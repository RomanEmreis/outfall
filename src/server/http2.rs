use std::{
    future::Future,
    error::Error,
    io
};
use bytes::{Bytes, BytesMut};
use h2::{server, RecvStream};
use h2::server::SendResponse;
use http::{Request, Response};
use tokio::net::TcpStream;

pub struct Connection {
    stream: TcpStream
}

impl Connection {
    pub fn new(tcp_stream: TcpStream) -> Self {
        Self { stream: tcp_stream }
    }

    pub async fn run<F, Fut>(self, service_fn: F) -> Result<(), Box<dyn Error + Send + Sync>>
    where
        F: Fn(Request<Bytes>) -> Fut + Send + Copy + 'static,
        Fut: Future<Output = io::Result<Response<Bytes>>> + Send + 'static
    {
        let mut connection = server::handshake(self.stream).await?;
        while let Some(result) = connection.accept().await {
            tokio::spawn(async move {
                let (request, respond) = result?;
                let request = Self::recv_stream_to_bytes(request).await?;
                let response = service_fn(request).await?;

                Self::send_response(respond, response).await
            });
        }

        Ok(())
    }

    /// Convert `http::Request<RecvStream>` to `http::Request<Bytes>`
    async fn recv_stream_to_bytes(req: Request<RecvStream>) -> Result<Request<Bytes>, Box<dyn Error + Send + Sync>> {
        let (parts, body_stream) = req.into_parts();
        
        let mut body = BytesMut::new();
        let mut stream = body_stream;

        while let Some(chunk) = stream.data().await {
            let chunk = chunk?;
            body.extend_from_slice(&chunk);
        }
        let body = body.freeze();
        Ok(Request::from_parts(parts, body))
    }

    /// Send `http::Response<Bytes>` as a response over `SendStream`
    async fn send_response(mut respond: SendResponse<Bytes>, response: Response<Bytes>) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (parts, body) = response.into_parts();
        let response_headers = Response::from_parts(parts, ());

        let mut send_stream = respond.send_response(response_headers, body.is_empty())?;
        if !body.is_empty() {
            send_stream.send_data(body, true)?;
        }

        Ok(())
    }
}