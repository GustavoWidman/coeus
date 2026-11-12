use std::process::{Command, Stdio};

use log::debug;

/// a desktop notification, best effort: `osascript` on macOS, `notify-send`
/// elsewhere
pub fn notify(title: &str, body: &str) {
    let mut command = match cfg!(target_os = "macos") {
        true => {
            let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!(
                "display notification \"{}\" with title \"{}\" sound name \"default\"",
                quote(body),
                quote(title)
            );
            let mut command = Command::new("osascript");
            command.args(["-e", &script]);
            command
        }
        false => {
            let mut command = Command::new("notify-send");
            command.args([title, body]);
            command
        }
    };

    let shown = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
    if let Err(error) = shown {
        debug!("could not show a desktop notification: {error}");
    }
}
