use nova_protocol::{NVP_MAGIC, NVP_VERSION};
use nova_server::{NovaServer, ServerConfig};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_malformed_protocol_resilience() {
    let tmp = tempdir().unwrap();
    let config = ServerConfig {
        port: 17555,
        data_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let server = NovaServer::new(config).unwrap();
    let shutdown = server.shutdown_handle();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // 1. Send invalid magic bytes: "XXXX"
    {
        let mut stream = TcpStream::connect("127.0.0.1:17555").await.unwrap();
        stream
            .write_all(b"XXXX\x00\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00")
            .await
            .unwrap();
        let mut buf = [0u8; 64];
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_millis(500),
            stream.read(&mut buf),
        )
        .await;
    }

    // 2. Send oversized declared frame length (100 MB)
    {
        let mut stream = TcpStream::connect("127.0.0.1:17555").await.unwrap();
        let mut bad_header = Vec::new();
        bad_header.extend_from_slice(&NVP_MAGIC);
        bad_header.extend_from_slice(&NVP_VERSION.to_be_bytes());
        bad_header.push(0x01); // Request
        bad_header.push(0x00); // Flags
        bad_header.extend_from_slice(&1u64.to_be_bytes());
        bad_header.extend_from_slice(&(100u32 * 1024 * 1024).to_be_bytes()); // 100 MB declared length

        stream.write_all(&bad_header).await.unwrap();
        let mut buf = [0u8; 64];
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_millis(500),
            stream.read(&mut buf),
        )
        .await;
    }

    // 3. Server must still be alive and responsive to valid clients!
    {
        let client = nova_client::NovaClient::connect("127.0.0.1:17555")
            .await
            .unwrap();
        assert!(client.ping().await.is_ok());
    }

    let _ = shutdown.send(());
}
