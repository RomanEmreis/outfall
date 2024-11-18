use bytes::{
    Bytes, 
    BytesMut
};
use http::{
    HeaderName,
    HeaderValue,
    Request,
    Response,
    Version
};
use tokio::{
    net::TcpStream,
    io::{
        AsyncReadExt,
        AsyncWriteExt,
        BufReader
    }
};
use std::{
    future::Future,
    io::{
        ErrorKind::{
            BrokenPipe,
            InvalidData,
            InvalidInput
        },
        Error
    }
};

pub struct Connection {
    reader: BufReader<TcpStream>
}

impl Connection {
    pub fn new(tcp_stream: TcpStream) -> Self {
        Self { reader: BufReader::new(tcp_stream) }
    }
    
    pub async fn run<F, Fut>(mut self, service_fn: F)
    where
        F: Fn(Request<Bytes>) -> Fut,
        Fut: Future<Output = std::io::Result<Response<Bytes>>>,
    {
        let mut buffer = BytesMut::with_capacity(4096);
        loop {
            match self.read(&mut buffer).await { 
                Ok(req) => {
                    match service_fn(req).await { 
                        Ok(response) => {
                            if let Err(err) = self.write(response).await {
                                if cfg!(debug_assertions) {
                                    eprintln!("Failed to write to socket: {:?}", err);
                                }
                                break;
                            }
                        },
                        Err(err) => {
                            if cfg!(debug_assertions) {
                                eprintln!("Error occurred handling request: {}", err);
                            }
                            break;
                        }
                    }

                },
                Err(err) => {
                    if cfg!(debug_assertions) {
                        eprintln!("Error occurred handling request: {}", err);
                    }
                    break;
                }
            }
        }
    }
    
    async fn read(&mut self, buffer: &mut BytesMut) -> std::io::Result<Request<Bytes>> {
        buffer.clear();
        loop {
            let bytes_read = self.reader.read_buf(buffer).await?;
            if bytes_read == 0 {
                return Err(Error::new(BrokenPipe, "Client closed the connection"));
            }

            let mut headers = [httparse::EMPTY_HEADER; 16];
            let mut req = httparse::Request::new(&mut headers);
            match req.parse(&buffer[..]) {
                Ok(httparse::Status::Complete(headers_size)) => {
                    let body = Bytes::copy_from_slice(&buffer[headers_size..]);

                    let method = req.method.ok_or_else(|| Error::new(InvalidData, "No method specified"))?;
                    let path = req.path.ok_or_else(|| Error::new(InvalidData, "No path specified"))?;

                    let mut builder = Request::builder()
                        .method(method)
                        .uri(path)
                        .version(Version::HTTP_11);

                    for header in req.headers {
                        let header_name = HeaderName::from_bytes(header.name.as_bytes())
                            .map_err(|e| 
                                Error::new(InvalidData, format!("Invalid header name: {}", e))
                            )?;
                        let header_value = HeaderValue::from_bytes(header.value)
                            .map_err(|e| 
                                Error::new(InvalidData, format!("Invalid header value: {}", e))
                            )?;
                        builder = builder.header(header_name, header_value);
                    }

                    return builder.body(body)
                        .map_err(|_| Error::new(InvalidInput, "Failed to build request"));
                }
                Ok(httparse::Status::Partial) => continue,
                Err(e) => return Err(Error::new(InvalidData, format!("Failed to parse request: {}", e)))
            }
        }
    }
    
    async fn write(&mut self, response: Response<Bytes>) -> std::io::Result<()> {
        let stream = self.reader.get_mut();
        let mut response_bytes = BytesMut::new();
        let status_line = format!(
            "HTTP/1.1 {} {}\r\n",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("unknown status")
        );
        response_bytes.extend_from_slice(status_line.as_bytes());

        let mut headers = response.headers().clone();
        if !headers.contains_key("content-length") {
            let content_length = response.body().len();
            headers.insert(
                "content-length",
                HeaderValue::from_str(&content_length.to_string()).unwrap(),
            );
        }

        let mut headers = response.headers().clone();
        if !headers.contains_key("content-type") {
            headers.insert(
                "content-type",
                HeaderValue::from_str("text/plain").unwrap(),
            );
        }
        
        for (key, value) in response.headers() {
            let header_value = match value.to_str() {
                Ok(v) => v,
                Err(_) => return Err(Error::new(InvalidData, "Invalid header value")),
            };
            let header = format!("{}: {}\r\n", key, header_value);
            response_bytes.extend_from_slice(header.as_bytes());
        }

        response_bytes.extend_from_slice(b"\r\n");
        if !response.body().is_empty() {
            response_bytes.extend_from_slice(response.body());
        }

        stream.write_all(&response_bytes).await?;
        stream.flush().await
    }
}
