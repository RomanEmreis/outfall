use std::io::Error;
use bytes::Bytes;
use http::{Request, Response};
use tokio::io;
use tokio::net::TcpListener;
use outfall::server::http1;

async fn request_handler(_req: Request<Bytes>) -> io::Result<Response<Bytes>> {
    Response::builder()
        .status(200)
        .body("Hello World!".into())
        .map_err(|_| Error::new(io::ErrorKind::Other, "Unable to create a response"))
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let tcp_listener = TcpListener::bind("127.0.0.1:7878").await?;

    loop {
        let (stream, _) = tcp_listener.accept().await?;

        tokio::spawn(async move {
            let conn = http1::Connection::new(stream);
            conn.run(request_handler).await;
        });
    }
}