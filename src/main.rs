use std::process::exit;
use std::format;
use std::convert::TryFrom;
use std::io::Write;
use log::{info, warn, error};
use clap::{App, Arg};


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
                warn!("args: invalid set port (must be a number in the range 0..65535, got {:?}) \
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
                    warn!("args: invalid set port (must be a number in the range 0..65535, got {:?}) \
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

    info!("server listening at {}:{}", ip, port);
}
