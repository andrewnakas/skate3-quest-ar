//! Optional Steam byte relay. The game never links this SDK or calls Steam Input.
mod directory;
use std::{
    collections::BTreeMap,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};
use steamworks::{
    Client, SteamId,
    networking_types::{NetworkingIdentity, SendFlags},
};
const CHANNEL: u32 = 38;
fn main() {
    if let Err(e) = run() {
        eprintln!("Steam relay: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "Expected parent-address, peer Steam ID (0 to host), session, IPC cookie".into(),
        );
    }
    let parent: SocketAddr = args[0].parse().map_err(|_| "Invalid parent")?;
    if !parent.ip().is_loopback() {
        return Err("IPC must be loopback".into());
    }
    let mut peer = args[1].parse::<u64>().map_err(|_| "Invalid peer")?;
    let mut session = args[2].parse::<u64>().map_err(|_| "Invalid session")?;
    let cookie = &args[3];
    let socket = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    socket.connect(parent).map_err(|e| e.to_string())?;
    skate_net::socket::configure(&socket).map_err(|e| e.to_string())?;
    let status = |s: &str| {
        let _ = socket.send(format!("SK8RELAY {cookie} {s}").as_bytes());
    };
    status("Starting Steam");
    let client = match Client::init_app(480) {
        Ok(c) => c,
        Err(e) => {
            status(&format!("ERROR Open Steam and sign in, then retry. {e}"));
            return Err(e.to_string());
        }
    };
    let own = client.user().steam_id().raw();
    if peer == own {
        status(
            "ERROR Two Steam players need different Steam accounts. Use local testing on one PC.",
        );
        return Err("Same Steam identity".into());
    }
    client.networking_utils().init_relay_network_access();
    // Remove our throughput ceiling without forcing a minimum send rate.
    // Steam still controls the connection; the game bounds in-flight data.
    // SAFETY: Steam is initialized and owns this interface for `client`'s lifetime.
    unsafe {
        let utils = steamworks::sys::SteamAPI_SteamNetworkingUtils_SteamAPI_v004();
        if utils.is_null()
            || !steamworks::sys::SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueInt32(
                utils,
                steamworks::sys::ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_SendRateMax,
                i32::MAX,
            )
        {
            return Err("Could not configure Steam model transfer bandwidth".into());
        }
    }
    let messages = client.networking_messages();
    let mut attempts = 0;
    let mut window = Instant::now();
    messages.session_request_callback(move |request| {
        if window.elapsed() > Duration::from_secs(10) {
            attempts = 0;
            window = Instant::now();
        }
        let id = request.remote().steam_id().map(|id| id.raw()).unwrap_or(0);
        if id != 0 && id != own && attempts < 20 {
            attempts += 1;
            request.accept();
        }
    });
    // Game admission owns the ten-player limit; this adapter only routes bytes.
    let mut peers: BTreeMap<u64, (Instant, i64)> = BTreeMap::new();
    if peer != 0 {
        peers.insert(peer, (Instant::now(), 0));
    }
    let cookie_value = u64::from_str_radix(cookie, 16).map_err(|_| "Invalid cookie")?;
    let ping = format!("PING {cookie}");
    let mut dropped = 0u64;
    let mut failures = 0u64;
    let mut heartbeat = Instant::now() - Duration::from_secs(2);
    let mut parent_seen = Instant::now();
    let mut buffer = [0u8; 1500];
    let mut directory = directory::Directory::new();
    let mut route = None;
    'running: loop {
        client.run_callbacks();
        for response in directory.poll(&client) {
            status(&format!(
                "EVENT {}",
                skate_net::directory::encode(&response)
            ));
        }
        if let Some(lobby) = directory.lobby {
            let next = (lobby.raw(), directory.owner);
            if directory.owner != 0 && route != Some(next) {
                session = lobby.raw();
                peer = if directory.owner == own {
                    0
                } else {
                    directory.owner
                };
                peers.clear();
                if peer != 0 {
                    peers.insert(peer, (Instant::now(), 0));
                }
                route = Some(next);
            }
        }
        if heartbeat.elapsed() > Duration::from_secs(1) {
            status(&format!("READY {own}"));
            let mut up = 0f32;
            let mut down = 0f32;
            let mut queue = 0i64;
            let mut loss = 0f32;
            let mut ping_ms = 0;
            // NetworkingMessages expires idle sessions; bound our routing/metrics cache too.
            peers.retain(|id, (seen, _)| seen.elapsed() <= Duration::from_secs(10) || *id == peer);
            for (&id, (_, queued)) in &mut peers {
                let (_, _, realtime) = messages.get_session_connection_info(
                    &NetworkingIdentity::new_steam_id(SteamId::from_raw(id)),
                );
                if let Some(r) = realtime {
                    up += r.out_bytes_per_sec();
                    down += r.in_bytes_per_sec();
                    ping_ms = ping_ms.max(r.ping());
                    // steamworks-rs calls this queued_send_bytes, but the SDK field is m_usecQueueTime.
                    *queued = r.queued_send_bytes().max(0) / 1000;
                    queue = queue.max(*queued);
                    for quality in [r.connection_quality_local(), r.connection_quality_remote()] {
                        if quality >= 0. {
                            loss = loss.max((1. - quality) * 100.);
                        }
                    }
                }
            }
            status(&format!(
                "STATS {} Steam up {:.1} / down {:.1} kB/s | ping {} ms | loss {:.1}% | queue {} ms | drops {} | errors {}",
                u8::from(queue > 20),
                up / 1000.,
                down / 1000.,
                ping_ms,
                loss,
                queue,
                dropped,
                failures
            ));
            heartbeat = Instant::now();
        }
        for _ in 0..4096 {
            match socket.recv(&mut buffer) {
                Ok(n) => {
                    if buffer[..n] == *ping.as_bytes() {
                        parent_seen = Instant::now();
                        continue;
                    }
                    if buffer[..n] == *format!("QUIT {cookie}").as_bytes() {
                        directory.leave(&client.matchmaking());
                        break 'running;
                    }
                    if let Ok(text) = std::str::from_utf8(&buffer[..n]) {
                        if let Some(json) = text.strip_prefix(&format!("CMD {cookie} ")) {
                            if let Some(request) = skate_net::directory::request(json) {
                                parent_seen = Instant::now();
                                if let Some(response) = directory.command(request, &client) {
                                    status(&format!(
                                        "EVENT {}",
                                        skate_net::directory::encode(&response)
                                    ));
                                }
                            }
                            continue;
                        }
                    }
                    if n < 16 || u64::from_le_bytes(buffer[..8].try_into().unwrap()) != cookie_value
                    {
                        continue;
                    }
                    let target = u64::from_le_bytes(buffer[8..16].try_into().unwrap());
                    let data = &buffer[16..n];
                    let Some((s, _, kind, _)) = skate_net::packed::envelope(data) else {
                        continue;
                    };
                    if s != session
                        || target == 0
                        || (peer != 0 && peer != target)
                        || (directory.lobby.is_some() && !directory.members.contains(&target))
                    {
                        continue;
                    }
                    parent_seen = Instant::now();
                    let state = matches!(kind, skate_net::packed::BODY | skate_net::packed::POSE);
                    // Do not discard packets based on the once-per-second
                    // queue sample. That creates long artificial loss bursts.
                    // Steam handles backpressure; state remains NoDelay, and
                    // bulk retries/ACK windows remain owned by skate-net.
                    let flags = if state {
                        SendFlags::UNRELIABLE_NO_DELAY
                    } else {
                        SendFlags::UNRELIABLE_NO_NAGLE
                    };
                    if let Err(e) = messages.send_message_to_user(
                        NetworkingIdentity::new_steam_id(SteamId::from_raw(target)),
                        flags,
                        data,
                        CHANNEL,
                    ) {
                        if matches!(e, steamworks::SteamError::Ignored) {
                            dropped += 1;
                        } else {
                            failures += 1;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        for message in messages.receive_messages_on_channel(CHANNEL, 4096) {
            let id = message
                .identity_peer()
                .steam_id()
                .map(|id| id.raw())
                .unwrap_or(0);
            let Some((s, _, kind, _)) = skate_net::packed::envelope(message.data()) else {
                continue;
            };
            if s != session
                || id == 0
                || id == own
                || (peer != 0 && id != peer)
                || (directory.lobby.is_some() && !directory.members.contains(&id))
            {
                continue;
            }
            if !peers.contains_key(&id) {
                if kind != skate_net::lobby::HELLO || peers.len() >= 12 {
                    continue;
                }
                peers.insert(id, (Instant::now(), 0));
            }
            peers.get_mut(&id).unwrap().0 = Instant::now();
            let mut packet = cookie_value.to_le_bytes().to_vec();
            packet.extend(id.to_le_bytes());
            packet.extend(message.data());
            if socket.send(&packet).is_err() {
                failures += 1;
            }
        }
        if parent_seen.elapsed() > Duration::from_secs(15) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    directory.leave(&client.matchmaking());
    Ok(())
}
