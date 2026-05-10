use mimalloc::MiMalloc;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream, UnixStream};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn main() -> std::io::Result<()> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9999);
    let upstreams: Vec<String> = std::env::var("UPSTREAMS")
        .unwrap_or_else(|_| "/run/sock/api1.sock,/run/sock/api2.sock".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if upstreams.is_empty() {
        eprintln!("rinha-lb: no upstreams");
        std::process::exit(1);
    }
    let upstreams: &'static [String] = Box::leak(upstreams.into_boxed_slice());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;

    rt.block_on(async move {
        let listener = build_listener(port).expect("build listener");
        eprintln!("rinha-lb: listening on :{}, upstreams={:?}", port, upstreams);
        loop {
            match listener.accept().await {
                Ok((client, _)) => {
                    let _ = client.set_nodelay(true);
                    tokio::spawn(handle(client, upstreams));
                }
                Err(e) => {
                    eprintln!("accept err: {}", e);
                }
            }
        }
    });
    Ok(())
}

fn build_listener(port: u16) -> std::io::Result<TcpListener> {
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse().unwrap();
    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    sock.set_reuse_address(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&addr.into())?;
    sock.listen(4096)?;
    TcpListener::from_std(sock.into())
}

async fn handle(mut client: TcpStream, upstreams: &'static [String]) {
    let idx = COUNTER.fetch_add(1, Ordering::Relaxed) % upstreams.len();
    let path = &upstreams[idx];

    let mut backend = match UnixStream::connect(path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("rinha-lb: connect {} err: {}", path, e);
            return;
        }
    };

    let _ = copy_bidirectional(&mut client, &mut backend).await;
}
