use skate_net::directory::{self, Command, Event, Request, Response, Row};
use std::{
    collections::BTreeSet,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
use steamworks::{
    Client, DistanceFilter, LobbyId, LobbyKey, LobbyType, Matchmaking, StringFilter,
    StringFilterKind,
};
enum ResultEvent {
    List(u64, usize, u64, u64, Result<Vec<LobbyId>, String>),
    Host(u64, u64, u64, String, Result<LobbyId, String>),
    Join(u64, u64, u64, Result<LobbyId, String>),
}
pub struct Directory {
    pub lobby: Option<LobbyId>,
    pub owner: u64,
    pub members: BTreeSet<u64>,
    tx: Sender<ResultEvent>,
    rx: Receiver<ResultEvent>,
    request: Option<u64>,
    pending: bool,
    began: Instant,
    cached: Option<Response>,
    last_owner: Instant,
}
impl Directory {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            lobby: None,
            owner: 0,
            members: BTreeSet::new(),
            tx,
            rx,
            request: None,
            pending: false,
            began: Instant::now(),
            cached: None,
            last_owner: Instant::now() - Duration::from_secs(1),
        }
    }
    pub fn leave(&mut self, mm: &Matchmaking) {
        if let Some(l) = self.lobby.take() {
            mm.leave_lobby(l);
        }
        self.owner = 0;
        self.members.clear();
    }
    fn finish(&mut self, id: u64, event: Event) -> Response {
        let r = Response { request: id, event };
        self.pending = false;
        self.cached = Some(r.clone());
        r
    }
    pub fn command(&mut self, r: Request, client: &Client) -> Option<Response> {
        if self.request == Some(r.id) {
            return self.cached.clone();
        }
        if self.pending {
            return Some(Response {
                request: r.id,
                event: Event::Error("Steam request already in progress".into()),
            });
        }
        self.request = Some(r.id);
        self.pending = true;
        self.cached = None;
        self.began = Instant::now();
        let mm = client.matchmaking();
        let tx = self.tx.clone();
        let id = r.id;
        match r.command {
            Command::Browse { page, map, physics } => {
                // Keep unrelated Spacewar lobbies out of Steam's bounded result set,
                // while accepting every version of our namespace.
                mm.add_request_lobby_list_string_filter(StringFilter(
                    LobbyKey::new("sk8game"),
                    "skate3rust-free-skate-v",
                    StringFilterKind::EqualToOrGreaterThan,
                ));
                mm.add_request_lobby_list_string_filter(StringFilter(
                    LobbyKey::new("sk8game"),
                    "skate3rust-free-skate-w",
                    StringFilterKind::LessThan,
                ));
                mm.set_request_lobby_list_distance_filter(DistanceFilter::Worldwide);
                mm.request_lobby_list(move |result| {
                    let _ = tx.send(ResultEvent::List(
                        id,
                        page,
                        map,
                        physics,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
            Command::Host {
                map,
                physics,
                label,
            } => {
                self.leave(&mm);
                mm.create_lobby(LobbyType::Public, 10, move |result| {
                    let _ = tx.send(ResultEvent::Host(
                        id,
                        map,
                        physics,
                        label,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
            Command::Join {
                lobby,
                map,
                physics,
            } => {
                self.leave(&mm);
                mm.join_lobby(LobbyId::from_raw(lobby), move |result| {
                    let _ = tx.send(ResultEvent::Join(
                        id,
                        map,
                        physics,
                        result
                            .map_err(|_| "Lobby unavailable, full, or Steam could not join".into()),
                    ));
                });
            }
        };
        None
    }
    fn owner_event(&mut self, mm: &Matchmaking, own: u64) -> Option<Event> {
        let l = self.lobby?;
        let owner = mm.lobby_owner(l).raw();
        self.members = mm.lobby_members(l).iter().map(|i| i.raw()).collect();
        if owner == 0 || !self.members.contains(&own) {
            return None;
        }
        self.owner = owner;
        Some(Event::Owner {
            lobby: l.raw(),
            owner,
            own,
        })
    }
    pub fn poll(&mut self, client: &Client) -> Vec<Response> {
        let mm = client.matchmaking();
        let own = client.user().steam_id().raw();
        let mut output = Vec::new();
        while let Ok(result) = self.rx.try_recv() {
            let id = match &result {
                ResultEvent::List(id, ..)
                | ResultEvent::Host(id, ..)
                | ResultEvent::Join(id, ..) => *id,
            };
            if self.request != Some(id) || !self.pending {
                match result {
                    ResultEvent::Host(_, _, _, _, Ok(l)) | ResultEvent::Join(_, _, _, Ok(l)) => {
                        mm.leave_lobby(l)
                    }
                    _ => (),
                };
                continue;
            }
            let event = match result {
                ResultEvent::List(_, page, _map, _physics, result) => match result {
                    Err(e) => Event::Error(e),
                    Ok(mut list) => {
                        list.retain(|&l| {
                            mm.lobby_data(l, "sk8game")
                                .is_some_and(|name| directory::is_game_lobby(&name))
                        });
                        list.sort_by_key(LobbyId::raw);
                        let total = list.len();
                        let page = page.min(total.saturating_sub(1) / directory::PAGE_SIZE);
                        let rows = list
                            .into_iter()
                            .skip(page * directory::PAGE_SIZE)
                            .take(directory::PAGE_SIZE)
                            .map(|l| Row {
                                id: l.raw(),
                                map: clean(
                                    &mm.lobby_data(l, "label")
                                        .unwrap_or_else(|| "Unknown map".into()),
                                ),
                                players: mm.lobby_member_count(l).min(10),
                                capacity: mm.lobby_member_limit(l).unwrap_or(10).min(10),
                                compatible: true,
                            })
                            .collect();
                        Event::Rows { page, total, rows }
                    }
                },
                ResultEvent::Host(_, map, physics, label, result) => match result {
                    Err(e) => Event::Error(e),
                    Ok(l) => {
                        let okay = mm.set_lobby_data(l, "map", &map.to_string())
                            && mm.set_lobby_data(l, "physics", &physics.to_string())
                            && mm.set_lobby_data(l, "label", &clean(&label))
                            && mm.set_lobby_data(l, "sk8game", directory::NAMESPACE);
                        if !okay {
                            mm.leave_lobby(l);
                            Event::Error("Steam could not publish lobby details".into())
                        } else {
                            self.lobby = Some(l);
                            self.owner_event(&mm, own)
                                .unwrap_or(Event::Error("Steam lobby owner unavailable".into()))
                        }
                    }
                },
                ResultEvent::Join(_, _map, _physics, result) => match result {
                    Err(e) => Event::Error(e),
                    Ok(l) => {
                        self.lobby = Some(l);
                        self.owner_event(&mm, own)
                            .unwrap_or(Event::Error("Steam lobby owner unavailable".into()))
                    }
                },
            };
            output.push(self.finish(id, event));
        }
        if self.pending && self.began.elapsed() > Duration::from_secs(15) {
            output.push(self.finish(
                self.request.unwrap(),
                Event::Error("Steam request timed out; refresh or retry".into()),
            ));
        }
        // Repeated notifications survive loopback datagram loss and track Steam owner election.
        if self.last_owner.elapsed() > Duration::from_millis(250) {
            if let Some(event) = self.owner_event(&mm, own) {
                output.push(Response { request: 0, event });
            }
            self.last_owner = Instant::now();
        }
        output
    }
}
fn clean(text: &str) -> String {
    directory::label(text)
}
