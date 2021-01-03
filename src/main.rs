use std::process::exit;
use std::format;
use std::convert::{TryFrom,Infallible};
use std::io::Write;
use std::net::{ToSocketAddrs, SocketAddr};
use log::{info, warn, error, debug};
use futures_util::future::try_join;
use clap::{App, Arg};
use http;
use tokio::net::TcpStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Body, Client, Method, Request, Response, Server};
use hyper::server::conn::AddrStream;


pub type HttpClient = Client<hyper::client::HttpConnector>;


#[tokio::main]
async fn main() {
    // setup logging
    env_logger::Builder::new()
        .format(|in_buf, record| {
            writeln!(in_buf,
                     "{} [{}] - {}",
                     chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
                     record.level(),
                     record.args()
            )
        })
        .filter(None, log::LevelFilter::Debug)
        .init();

    // setup argument parser
    const NAME: &'static str = env!("CARGO_PKG_NAME");
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    let env_app: String = NAME.to_uppercase().replace('-', "_");
    let env_ip = format!("{}{}", env_app, "_IP");
    let env_port = format!("{}{}", env_app, "_PORT");

    let arg_matches = App::new(NAME)
        .version(VERSION)
        .arg(Arg::with_name("config")
            .long("config")
            .short("c")
            .default_value("config.yaml")
            .help("Sets a config for server")
        )
        .arg(Arg::with_name("ip")
            .long("ip")
            .env(&*env_ip)
            .help("Sets a ip address for server")
        )
        .arg(Arg::with_name("port")
            .long("port")
            .short("p")
            .env(&*env_port)
            .help("Sets a port for server")
        )
        .get_matches();

    const DEFAULT_IP: &'static str = "127.0.0.1";
    let mut ip = String::from(arg_matches.value_of("ip").unwrap_or(DEFAULT_IP));
    const DEFAULT_PORT: u16 = 8080;
    let mut port: u16 = match arg_matches.value_of("port") {
        Some(v) => match v.parse::<u16>() {
            Ok(v) => v,
            Err(_) => {
                warn!("invalid set port in args (must be a number in the range 0..65535, got {:?}) \
                       will be changed to default value ({})", v, DEFAULT_PORT);
                DEFAULT_PORT
            }
        },
        None => DEFAULT_PORT
    };

    // read config
    let config_path = arg_matches.value_of("config").unwrap();
    let config_file = match std::fs::File::open(config_path) {
        Ok(v) => v,
        Err(e) => {
            error!("can not open config file {:?}; err = {:?}", config_path, e);
            exit(78);
        }
    };
    let config: serde_yaml::Value = match serde_yaml::from_reader(config_file) {
        Ok(v) => v,
        Err(e) => {
            error!("can not open config file {:?}; err = {:?}", config_path, e);
            exit(78);
        }
    };

    if !arg_matches.is_present("ip") {
        ip = match config.get("ip") {
            Some(v) => serde_yaml::from_value(v.clone()).unwrap(),
            None => ip
        };
    }

    if !arg_matches.is_present("port") {
        let p = match &config.get("port") {
            Some(v) => {
                let p = match v {
                    &serde_yaml::Value::Number(v) => {
                        match &v.as_u64() {
                            Some(v) => match u16::try_from(v.clone()) {
                                Ok(v) => Some(v),
                                Err(_) => None
                            },
                            None => None
                        }
                    },
                    _ => None
                };
                if p.is_none() {
                    warn!("invalid set port in config (must be a number in the range 0..65535, got {:?}) \
                           will be changed to default value ({})", v, port);
                }
                p
            }
            None => None
        };
        if !p.is_none() {
            port = p.unwrap();
        }
    }

    let addr = match to_addr(format!("{}:{}", ip, port)) {
        Some(v) => v,
        None => {
            error!("can not resolve server address {}:{}", ip, port);
            exit(78);
        }
    };
    let client = HttpClient::new();

    let make_service = make_service_fn(move |conn: &AddrStream| {
        let client = client.clone();
        let peer = conn.remote_addr();
        async move { Ok::<_, Infallible>(service_fn(move |req| proxy(client.clone(), req, peer))) }
    });

    let server = Server::bind(&addr).serve(make_service);

    info!("server listening at {}", addr);

    if let Err(e) = server.await {
        error!("server crashed; err = {:?}", e);
    }
}

fn to_addr(host: String) -> Option<SocketAddr> {

    let mut addrs_iter = match host.to_socket_addrs() {
        Ok(v) => v,
        Err(_) => {
            return None;
        }
    };

    match addrs_iter.next() {
        Some(v) => Some(v),
        None => None
    }

}

async fn proxy(client: HttpClient, req: Request<Body>, peer: SocketAddr) -> Result<Response<Body>, hyper::Error> {
    info!("client {:?}: connected", peer);
    debug!("client {:?}: request = {:?}", peer, req);

    if Method::CONNECT == req.method() {
        // Creates a tunnel between the client and the remote server
        //
        //            Client                     Forward Proxy                    Server
        //               CONNECT request ------------->
        //                                             CONNECT request ------------->
        //                                             <-------- CONNECT response: OK
        //               <-------- CONNECT response: OK
        //               <--------------------- SSL negotiations ------------------->
        //               ------------------------ HTTP request --------------------->
        //               <----------------------- HTTP response -------------------->
        //
        // Received an HTTP request like:
        // ```
        // CONNECT www.domain.com:443 HTTP/1.1
        // Host: www.domain.com:443
        // Proxy-Connection: Keep-Alive
        // ```
        //
        // When HTTP method is CONNECT we should return an empty body
        // then we can eventually upgrade the connection and talk a new protocol.
        //
        // Note: only after client received an empty body with STATUS_OK can the
        // connection be upgraded, so we can't return a response inside
        // `on_upgrade` future.
        //
        let uri = req.uri();
        let addr = match uri.authority() {
            Some(v) => {
                let host = v.to_string();
                to_addr(host)
            },
            None => None
        };
        if addr.is_some() {
            let addr = addr.unwrap();
            error!("client {:?}: upstream remote uri {:?}", peer, uri);
            tokio::task::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        if let Err(e) = tunnel(upgraded, addr, peer).await {
                            error!("client {:?}: server io error; err = {:?}", peer, e);
                        };
                        info!("client {:?}: connection closed", peer);
                    }
                    Err(e) => error!("client {:?}: upgrade error; err = {:?}", peer, e),
                }
            });
            Ok(Response::new(Body::empty()))
        } else {
            error!("client {:?}: cannot resolve remote uri {:?}", peer, uri);
            let mut resp = Response::new(Body::from(format!("cannot resolve remote uri {:?}", uri)));
            *resp.status_mut() = http::StatusCode::BAD_REQUEST;
            Ok(resp)
        }
    } else {
        client.request(req).await.and_then(|resp| {
            info!("client {:?}: connection closed", peer);
            Ok(resp)
        })
    }
}


async fn tunnel(upgraded: Upgraded, addr: SocketAddr, peer: SocketAddr) -> std::io::Result<()> {
    // Connect to remote server
    let mut server = TcpStream::connect(addr).await?;

    // Proxying data
    let amounts = {
        let (mut server_rd, mut server_wr) = server.split();
        let (mut client_rd, mut client_wr) = tokio::io::split(upgraded);

        let client_to_server = tokio::io::copy(&mut client_rd, &mut server_wr);
        let server_to_client = tokio::io::copy(&mut server_rd, &mut client_wr);

        try_join(client_to_server, server_to_client).await
    };

    // Print message when done
    match amounts {
        Ok((from_client, from_server)) => {
            debug!("client {:?}: {} - wrote {} bytes and received {} bytes", peer, addr, from_client, from_server);
        }
        Err(e) => {
            error!("client {:?}: tunnel error err = {:?}", peer, e);
        }
    };
    Ok(())
}
