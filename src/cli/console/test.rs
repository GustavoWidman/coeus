use std::{os::unix::process::ExitStatusExt, process::Command};

use super::Console;
use crate::ui::checklist::Checklist;

/// the terminal's state is the process's, so this test runs itself again
/// as a child process, which starts a live console and is sent SIGTERM
#[tokio::test]
async fn a_terminated_run_gives_the_cursor_back_and_says_how_it_ended() {
    const CHILD: &str = "COEUS_TEST_TERMINATED_CONSOLE";
    if std::env::var_os(CHILD).is_none() {
        let name =
            "cli::console::test::a_terminated_run_gives_the_cursor_back_and_says_how_it_ended";
        let child = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);

        assert_eq!(child.status.signal(), None, "{stdout}");
        assert_eq!(child.status.code(), Some(143), "{stdout}");
        let hidden = stdout.find("\x1b[?25l").expect("the cursor was hidden");
        let shown = stdout.rfind("\x1b[?25h").expect("the cursor was shown");
        assert!(hidden < shown, "{stdout:?}");
        return;
    }

    let _console = Console::start(true, Checklist::new("nova".into(), Vec::new())).unwrap();
    let pid = std::process::id().to_string();
    Command::new("kill").args(["-TERM", &pid]).status().unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    unreachable!("SIGTERM ends the process");
}
