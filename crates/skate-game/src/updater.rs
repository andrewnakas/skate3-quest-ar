//! The packaged helper owns networking/UI/staging. Only its ready signal exits Bevy.
use bevy::prelude::*;
use std::{
    path::PathBuf,
    process::{Child, Command},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Resource, Default)]
pub(crate) struct Updater {
    child: Option<Child>,
    signal: Option<PathBuf>,
    temporary: Option<PathBuf>,
}

fn helper_command(recover: bool, automatic: bool) -> Result<(Command, PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = exe.parent().ok_or("Missing program directory")?;
    if !root.join("release.json").is_file() {
        return Err("Updates are available in packaged releases.".into());
    }
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("skate-update-{}-{unique}", std::process::id()));
    std::fs::create_dir(&temp).map_err(|e| e.to_string())?;
    let helper = temp.join("skate3update.exe");
    std::fs::copy(root.join("support/skate3update.exe"), &helper).map_err(|e| e.to_string())?;
    let signal = temp.join("ready");
    let request = temp.join("request.json");
    let data = serde_json::json!({
        "revision": env!("SKATE_RELEASE_REVISION"), "build": env!("SKATE_RELEASE_BUILD"),
        "root": root, "signal": signal, "automatic": automatic, "recover": recover,
        "cwd": std::env::current_dir().map_err(|e| e.to_string())?,
        "args": std::env::args().skip(1).collect::<Vec<_>>()
    });
    std::fs::write(
        &request,
        serde_json::to_vec(&data).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut command = Command::new(helper);
    command.arg("--request").arg(request);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok((command, signal, temp))
}

/// Local recovery happens before the supervisor opens the executable again.
pub(crate) fn recover() -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if !exe
        .parent()
        .is_some_and(|p| p.join(".update-transaction/journal.json").exists())
    {
        return Ok(false);
    }
    let (mut command, _, _) = helper_command(true, false)?;
    command.spawn().map_err(|e| e.to_string())?;
    Ok(true)
}

impl Updater {
    pub(crate) fn open(&mut self, automatic: bool) -> String {
        if self.child.is_some() {
            return "Updates window is already open.".into();
        }
        match helper_command(false, automatic).and_then(|(mut cmd, signal, temp)| {
            cmd.spawn()
                .map(|child| (child, signal, temp))
                .map_err(|e| e.to_string())
        }) {
            Ok((child, signal, temp)) => {
                self.child = Some(child);
                self.signal = Some(signal);
                self.temporary = Some(temp);
                "Updates opened in a separate window.".into()
            }
            Err(error) => error,
        }
    }
}

pub(crate) struct UpdaterPlugin;
impl Plugin for UpdaterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Updater>()
            .add_systems(Startup, |mut updater: ResMut<Updater>| {
                updater.open(true);
            })
            .add_systems(Update, poll);
    }
}
fn poll(mut updater: ResMut<Updater>, mut exit: MessageWriter<AppExit>) {
    if updater.signal.as_ref().is_some_and(|p| p.is_file()) {
        updater.signal = None;
        // Success is intentional: the crash supervisor must not report an update.
        exit.write(AppExit::Success);
    }
    if updater
        .child
        .as_mut()
        .is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))))
    {
        updater.child = None;
        updater.signal = None;
        if let Some(temp) = updater.temporary.take() {
            let _ = std::fs::remove_dir_all(temp);
        }
    }
}
