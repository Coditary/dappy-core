mod common;

use std::process::Stdio;
use std::time::Duration;

use common::lock_tests;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[tokio::test]
async fn repl_plain_commands_execute() {
    let _guard = lock_tests().await;
    let mut child = Command::new(common::cli_bin())
        .args(["debug", "repl", "--adapter", "fake", "main.py"])
        .env("PATH", common::path_with_adapter())
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn plain repl");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    let commands = [
        "help\n",
        "info break\n",
        "break /fake/main.py:1\n",
        "info break\n",
        "catch uncaught\n",
        "info catch\n",
        "set x = 99\n",
        "bt\n",
        "locals\n",
        "scopes\n",
        "status\n",
        "sync\n",
        "skip list\n",
        "skip add /vendor/\n",
        "capabilities\n",
        "thread 1\n",
        "frame 1\n",
        "p 1 + 1\n",
        "clear /fake/main.py:1\n",
        "catch clear\n",
        "next\n",
        "quit\n",
    ];

    for command in commands {
        stdin.write_all(command.as_bytes()).await.expect("write cmd");
        stdin.flush().await.expect("flush cmd");
        read_until_prompt(&mut reader).await;
    }

    let status = timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("repl exit timeout")
        .expect("wait repl");
    assert!(status.success() || status.code() == Some(0));
}

async fn read_until_prompt(reader: &mut BufReader<tokio::process::ChildStdout>) {
    let mut line = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        line.clear();
        match timeout(Duration::from_millis(500), reader.read_line(&mut line)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(_)) => {
                if line.contains("dap>") {
                    break;
                }
            }
            _ => continue,
        }
    }
}
