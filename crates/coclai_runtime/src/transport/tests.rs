use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use super::*;

fn shell_spec(script: &str) -> StdioProcessSpec {
    let mut spec = StdioProcessSpec::new("sh");
    spec.args = vec!["-c".to_owned(), script.to_owned()];
    spec
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_zero_capacity_channels() {
    let err = match StdioTransport::spawn(
        shell_spec("cat"),
        StdioTransportConfig {
            read_channel_capacity: 0,
            write_channel_capacity: 16,
        },
    )
    .await
    {
        Ok(_) => panic!("must reject zero read channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let err = match StdioTransport::spawn(
        shell_spec("cat"),
        StdioTransportConfig {
            read_channel_capacity: 16,
            write_channel_capacity: 0,
        },
    )
    .await
    {
        Ok(_) => panic!("must reject zero write channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn writer_and_reader_roundtrip() {
    let mut transport = StdioTransport::spawn(shell_spec("cat"), StdioTransportConfig::default())
        .await
        .expect("spawn");
    let mut read_rx = transport.take_read_rx().expect("take rx");
    let write_tx = transport.write_tx().expect("take tx");

    write_tx
        .send(json!({"method":"ping","params":{"n":1}}))
        .await
        .expect("send #1");
    write_tx
        .send(json!({"method":"pong","params":{"n":2}}))
        .await
        .expect("send #2");
    drop(write_tx);

    let first = timeout(Duration::from_secs(2), read_rx.recv())
        .await
        .expect("recv timeout #1")
        .expect("stream closed #1");
    let second = timeout(Duration::from_secs(2), read_rx.recv())
        .await
        .expect("recv timeout #2")
        .expect("stream closed #2");

    assert_eq!(first["method"], "ping");
    assert_eq!(second["method"], "pong");

    drop(read_rx);
    let joined = transport.join().await.expect("join");
    assert!(joined.exit_status.success());
    assert_eq!(joined.malformed_line_count, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn reader_skips_malformed_lines() {
    let script = r#"printf '%s\n' '{"method":"ok"}' 'not-json' '{"id":1,"result":{}}' '{broken'"#;
    let mut transport = StdioTransport::spawn(shell_spec(script), StdioTransportConfig::default())
        .await
        .expect("spawn");
    let mut read_rx = transport.take_read_rx().expect("take rx");

    let mut parsed = Vec::new();
    while let Some(msg) = timeout(Duration::from_secs(2), read_rx.recv())
        .await
        .expect("recv timeout")
    {
        parsed.push(msg);
    }

    assert_eq!(parsed.len(), 2);
    assert_eq!(transport.malformed_line_count(), 2);

    drop(read_rx);
    let joined = transport.join().await.expect("join");
    assert!(joined.exit_status.success());
    assert_eq!(joined.malformed_line_count, 2);
}

#[tokio::test(flavor = "current_thread")]
async fn reader_survives_100k_lines_stream() {
    let script = r#"
i=0
while [ "$i" -lt 100000 ]; do
  printf '{"method":"tick","params":{"n":%s}}\n' "$i"
  i=$((i+1))
done
"#;
    let mut transport = StdioTransport::spawn(shell_spec(script), StdioTransportConfig::default())
        .await
        .expect("spawn");
    let mut read_rx = transport.take_read_rx().expect("take rx");

    let mut count = 0usize;
    while let Some(_msg) = timeout(Duration::from_secs(20), read_rx.recv())
        .await
        .expect("recv timeout")
    {
        count += 1;
    }

    assert_eq!(count, 100_000);
    assert_eq!(transport.malformed_line_count(), 0);

    drop(read_rx);
    let joined = transport.join().await.expect("join");
    assert!(joined.exit_status.success());
    assert_eq!(joined.malformed_line_count, 0);
}
