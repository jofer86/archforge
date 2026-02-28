//! Integration tests for the WebSocket transport.
//!
//! These tests spin up a real WebSocket server and client to verify
//! that data actually flows over the network correctly. Unlike unit
//! tests (which test logic in isolation), integration tests verify
//! that all the pieces work together.
//!
//! We use `tokio::test` because these tests are async — they need
//! the Tokio runtime to drive the futures (accept, connect, send, recv).

#[cfg(feature = "websocket")]
mod websocket {
    use arcforge_transport::{Connection, Transport, WebSocketTransport};

    /// Helper: connects a tokio-tungstenite client to the given address.
    /// Returns the raw WebSocket stream for sending/receiving from the
    /// client side.
    async fn connect_client(
        addr: &str,
    ) -> tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    > {
        let url = format!("ws://{addr}");
        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("client should connect");
        ws
    }

    #[tokio::test]
    async fn test_websocket_accept_and_send_receive() {
        // Spin up a WebSocket server on a random port.
        // "127.0.0.1:0" tells the OS to pick an available port.
        let transport = WebSocketTransport::bind("127.0.0.1:0")
            .await
            .expect("should bind");

        // We need the actual port the OS assigned so the client
        // can connect to it. We get it from the underlying listener.
        // But our API doesn't expose it, so we'll use a known port.
        // Actually, let's bind to a specific port for simplicity.
        drop(transport);
        let mut transport = WebSocketTransport::bind("127.0.0.1:19876")
            .await
            .expect("should bind");

        // Spawn the accept in a background task so we can connect
        // a client concurrently. `tokio::spawn` runs the future on
        // the Tokio runtime without blocking the current task.
        let server_handle = tokio::spawn(async move {
            transport.accept().await.expect("should accept")
        });

        // Connect a client.
        let mut client_ws = connect_client("127.0.0.1:19876").await;

        // Get the server-side connection.
        let server_conn = server_handle.await.expect("task should complete");

        // Verify the connection has a valid ID.
        assert!(server_conn.id().into_inner() > 0);

        // --- Server sends, client receives ---
        server_conn
            .send(b"hello from server")
            .await
            .expect("send should succeed");

        use futures_util::StreamExt;
        let msg = client_ws.next().await.unwrap().unwrap();
        assert_eq!(
            msg.into_data().as_ref(),
            b"hello from server",
        );

        // --- Client sends, server receives ---
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        client_ws
            .send(Message::Binary(b"hello from client".to_vec().into()))
            .await
            .unwrap();

        let received = server_conn
            .recv()
            .await
            .expect("recv should succeed")
            .expect("should have data");
        assert_eq!(received, b"hello from client");

        // --- Clean close ---
        server_conn.close().await.expect("close should succeed");
    }

    #[tokio::test]
    async fn test_websocket_recv_returns_none_on_client_close() {
        let mut transport = WebSocketTransport::bind("127.0.0.1:19877")
            .await
            .expect("should bind");

        let server_handle = tokio::spawn(async move {
            transport.accept().await.expect("should accept")
        });

        let client_ws = connect_client("127.0.0.1:19877").await;
        let server_conn = server_handle.await.unwrap();

        // Client closes the connection.
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        let mut client_ws = client_ws;
        client_ws.send(Message::Close(None)).await.unwrap();

        // Server should see None (clean close).
        let result = server_conn.recv().await.expect("recv should not error");
        assert!(result.is_none(), "should return None on client close");
    }
}
