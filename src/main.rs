#![recursion_limit = "1024"]

#[macro_use] extern crate error_chain;
#[macro_use] extern crate log;
extern crate env_logger;
#[macro_use] extern crate clap;
extern crate chrono;
extern crate rouille;
#[cfg(feature="update")]
extern crate self_update;
extern crate walkdir;

use std::env;
use std::time;
use std::fs;
use std::io::{self, Write};
use chrono::Local;
use clap::{Arg, App, SubCommand, ArgMatches};
use rouille::{Request, Response};
use rouille::proxy::ProxyConfig;


error_chain! {
    foreign_links {
        Io(io::Error);
        LogInit(log::SetLoggerError);
        Proxy(rouille::proxy::FullProxyError);
        SelfUpdate(self_update::errors::Error) #[cfg(feature="update")];
        StripPathPrefix(std::path::StripPrefixError);
    }
    errors {
        UrlPrefix(s: String) {
            description("Failed stripping url prefix")
            display("UrlPrefix Error: {}", s)
        }
        DoesNotExist(s: String) {
            description("Does not exist")
            display("DoesNotExist: {}", s)
        }
    }
}


#[derive(Debug, Clone)]
struct FileConfig {
    url: String,
    file_path: String,
    content_type: String,
}


#[derive(Debug, Clone)]
struct StaticConfig {
    prefix: String,
    directory: String,
}


#[derive(Debug, Clone)]
struct SubProxyConfig {
    prefix: String,
    proxy: ProxyConfig<String>,
}


fn proxy_request(request: &Request,
                 file_configs: &[FileConfig],
                 static_configs: &[StaticConfig],
                 subproxy_configs: &[SubProxyConfig],
                 proxy_config: &ProxyConfig<String>) -> Result<Response> {
    let url = request.url();
    for config in file_configs {
        if url == config.url {
            let file = fs::File::open(&config.file_path)?;
            return Ok(rouille::Response::from_file(config.content_type.clone(), file))
        }
    }
    for config in static_configs {
        if url.starts_with(&config.prefix) {
            let asset_request = request.remove_prefix(&config.prefix)
                .ok_or_else(|| ErrorKind::UrlPrefix(format!("Failed stripping prefix: {}", &config.prefix)))?;
            let resp = rouille::match_assets(&asset_request, &config.directory);
            if resp.is_success() { return Ok(resp) }
        }
    }
    for config in subproxy_configs {
        if url.starts_with(&config.prefix) {
            return Ok(rouille::proxy::full_proxy(&request, config.proxy.clone())?)
        }
    }
    Ok(rouille::proxy::full_proxy(&request, proxy_config.clone())?)
}


fn proxy_service(addr: &str,
           proxy_config: ProxyConfig<String>,
           subproxy_configs: Vec<SubProxyConfig>,
           static_configs: Vec<StaticConfig>,
           file_configs: Vec<FileConfig>) -> Result<()> {
    info!("** Serving on {:?} **", addr);
    info!("** Proxying to {:?} **", proxy_config.addr);
    info!("** Setting `Host` header: {:?} **", proxy_config.replace_host.as_ref().expect("missing replace_host"));
    info!("** Serving exact files: {:?} **", file_configs);
    info!("** Serving static dirs: {:?} **", static_configs);
    info!("** Serving sub-proxies: {:?} **", subproxy_configs);
    rouille::start_server(&addr, move |request| {
        let proxy_config        = &proxy_config;
        let subproxy_configs    = &subproxy_configs;
        let static_configs      = &static_configs;
        let file_configs        = &file_configs;

        let now = Local::now().format("%Y-%m-%d %H:%M%S");
        let log_ok = |req: &rouille::Request, resp: &rouille::Response, elap: time::Duration| {
            let ms = (elap.as_secs() * 1_000) as f32 + (elap.subsec_nanos() as f32 / 1_000_000.);
            info!("[{}] {} {} -> {} ({}ms)", now, req.method(), req.raw_url(), resp.status_code, ms)
        };
        let log_err = |req: &rouille::Request, elap: time::Duration| {
            let ms = (elap.as_secs() * 1_000) as f32 + (elap.subsec_nanos() as f32 / 1_000_000.);
            info!("[{}] Handler Panicked: {} {} ({}ms)", now, req.method(), req.raw_url(), ms)
        };
        // dispatch and handle errors
        rouille::log_custom(request, log_ok, log_err, move || {
            match proxy_request(request, file_configs, static_configs, subproxy_configs, proxy_config) {
                Ok(resp) => resp,
                Err(e) => {
                    error!("ProxyServiceError: {}", e);
                    Response::text("Something went wrong").with_status_code(500)
                }
            }
        })
    });
}


fn to_html(links: &[String]) -> String {
    let mut hrefs = String::new();
    for link in links[1..].iter() {
        hrefs.push_str(&format!("<li><a href=\"{link}\">{link}</a></li>\n", link=link));
    }
    let mut cur_dir = links[0].to_string();
    if !cur_dir.ends_with("/") { cur_dir.push('/'); }
    format!("<meta charset=\"UTF-8\">\
            <html><body>\
                <h1>{}</h1>\
                <ul>{}</ul>\
            </body></html>", cur_dir, hrefs)
}

fn fs_request(request: &Request, base_dir: &std::path::Path) -> Result<Response> {
    let resp = rouille::match_assets(&request, base_dir);
    if resp.is_success() { return Ok(resp.with_unique_header("Content-Type", "text/plain;charset=ISO-8859-1")); }

    let path = base_dir.join(std::path::Path::new(&request.url().trim_left_matches("/")));
    let path = path.canonicalize()
        .map_err(|_| ErrorKind::DoesNotExist(path.display().to_string()))?;
    if !path.starts_with(base_dir) {
        bail!("Attempt to access parent directories");
    }
    let walker = walkdir::WalkDir::new(&path).max_depth(1).into_iter();
    let mut links = vec![];
    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();
        let is_dir = path.is_dir();
        let mut path = String::from("/") + &path.strip_prefix(base_dir)?.display().to_string();
        if is_dir && !path.ends_with("/") { path.push('/'); }
        links.push(path);
    }
    let html = to_html(&links);
    Ok(Response::html(html))
}

fn fs_service(addr: &str, base_dir: &std::path::Path) -> Result<()> {
    info!("** Serving on {:?} **", addr);
    info!("** Serving from: {:?} **", base_dir);
    let base_dir = base_dir.to_owned();
    rouille::start_server(addr, move |request| {
        let base_dir = &base_dir;
        let now = Local::now().format("%Y-%m-%d %H:%M%S");
        let log_ok = |req: &rouille::Request, resp: &rouille::Response, elap: time::Duration| {
            let ms = (elap.as_secs() * 1_000) as f32 + (elap.subsec_nanos() as f32 / 1_000_000.);
            info!("[{}] {} {} -> {} ({}ms)", now, req.method(), req.raw_url(), resp.status_code, ms)
        };
        let log_err = |req: &rouille::Request, elap: time::Duration| {
            let ms = (elap.as_secs() * 1_000) as f32 + (elap.subsec_nanos() as f32 / 1_000_000.);
            info!("[{}] Handler Panicked: {} {} ({}ms)", now, req.method(), req.raw_url(), ms)
        };
        // dispatch and handle errors
        rouille::log_custom(request, log_ok, log_err, move || {
            let base_dir = &base_dir;
            match fs_request(request, &base_dir) {
                Ok(resp) => resp,
                Err(e) => {
                    error!("FsServiceError: {}", e);
                    match e.kind() {
                        &ErrorKind::DoesNotExist(_) => {
                            Response::text("Not Found").with_status_code(404)
                        }
                        _ => {
                            Response::text("Something went wrong").with_status_code(500)
                        }
                    }
                }
            }
        })
    });
}


fn run() -> Result<()> {
    let matches = App::new("Proxy")
        .version(crate_version!())
        .author("James K. <james.kominick@gmail.com>")
        .about("Repository: https://github.com/jaemk/proxy\n\
                Command-line proxy server for testing and development purposes. \
                Supports proxying requests and serving static files.")
        .subcommand(SubCommand::with_name("self")
            .about("Self referential things")
            .subcommand(SubCommand::with_name("update")
                .about("Update to the latest binary release, replacing this binary")
                .arg(Arg::with_name("no_confirm")
                     .help("Skip download/update confirmation")
                     .long("no-confirm")
                     .short("y")
                     .takes_value(false))
                .arg(Arg::with_name("quiet")
                     .help("Suppress unnecessary download output (progress bar)")
                     .long("quiet")
                     .short("q")
                     .takes_value(false))))
        .subcommand(SubCommand::with_name("fs")
            .about("Run a file server in the specified directory")
            .arg(Arg::with_name("public")
                 .display_order(1)
                 .long("public")
                 .help("Listen on `0.0.0.0` instead of `localhost`"))
            .arg(Arg::with_name("debug")
                 .display_order(2)
                 .help("Print debug info")
                 .long("debug")
                 .takes_value(false))
            .arg(Arg::with_name("port")
                 .display_order(0)
                 .help("Port to listen on")
                 .long("port")
                 .short("p")
                 .takes_value(true)
                 .default_value("3000"))
            .arg(Arg::with_name("directory")
                 .help("Directory to serve root request from.\n\
                        E.g. Start a file server in the current dir:\n\
                        `proxy fs .`")
                 .takes_value(true)
                 .required(true)))
        .subcommand(SubCommand::with_name("serve")
            .about("Run a proxy server, serving assets in ascending priority")
            .arg(Arg::with_name("public")
                 .display_order(1)
                 .long("public")
                 .help("Listen on `0.0.0.0` instead of `localhost`"))
            .arg(Arg::with_name("debug")
                 .display_order(2)
                 .help("Print debug info")
                 .long("debug")
                 .takes_value(false))
            .arg(Arg::with_name("port")
                 .display_order(0)
                 .help("Port to listen on")
                 .long("port")
                 .short("p")
                 .takes_value(true)
                 .default_value("3000"))
            .arg(Arg::with_name("exact-file")
                 .display_order(1)
                 .help("[- Priority 1 -] Url of direct-file-requests and the associated file to return. \
                        Formatted as `<exact-url>,<file-path>,<content-type>`.\n\
                        E.g. Return `static/index.html` for requests to `/`:\n    \
                        `--file /,static/index.html,text/html`\n\
                        Note, this argument can be provided multiple times.")
                 .long("file")
                 .short("f")
                 .takes_value(true)
                 .multiple(true)
                 .number_of_values(1))
            .arg(Arg::with_name("static-asset")
                 .display_order(2)
                 .help("[- Priority 2 -] Url prefix of static-asset-requests and the associated directory to serve files from. \
                        The url prefix will be stripped before looking in the specified directory. \
                        Formatted as `<url-prefix>,<directory>`.\n\
                        E.g. Serve requests starting with `/static/` from the relative directory `static`:\n    \
                        `--static /static/,static`\n\
                        Note, this argument can be provided multiple times.")
                 .long("static")
                 .short("s")
                 .takes_value(true)
                 .multiple(true)
                 .number_of_values(1))
            .arg(Arg::with_name("sub-proxy")
                 .display_order(3)
                 .help("[- Priority 3 -] Url prefix of sub-proxy-requests and the address to route requests to. \
                        The url will not be altered when proxied. \
                        Formatted as `<url-prefix>,<address>`.\n\
                        E.g. Proxy requests starting with `/api/` to `localhost:4500` instead of \
                        the \"main\" proxy:\n    \
                        `--sub-proxy /api/,localhost:4500`\n\
                        Note, this argument can be provided multiple times.")
                 .long("sub-proxy")
                 .short("P")
                 .takes_value(true)
                 .multiple(true)
                 .number_of_values(1))
            .arg(Arg::with_name("main-proxy")
                 .display_order(4)
                 .help("[- Priority 4 -] Address to proxy requests to. Formatted as <hostname>:<port>, e.g. `localhost:3002`")
                 .takes_value(true)
                 .required(true)))
        .get_matches();

    env::set_var("LOG", "info");
    if matches.is_present("debug") {
        env::set_var("LOG", "debug");
    }

    env_logger::Builder::new()
        .format(|buf, record| {
            writeln!(buf, "{} [{}] - [{}] -> {}",
                Local::now().format("%Y-%m-%d_%H:%M:%S"),
                record.level(),
                record.module_path().unwrap_or("<unknown>"),
                record.args()
                )
            })
    .parse(&env::var("LOG").unwrap_or_default())
    .init();

    match matches.subcommand() {
        ("self", Some(matches)) => {
            match matches.subcommand() {
                ("update", Some(matches)) => {
                    update(&matches)?;
                }
                _ => eprintln!("proxy: see `--help`"),
            }
            return Ok(())
        }
        ("fs", Some(matches)) => {
            let host = if matches.is_present("public") { "0.0.0.0" } else { "localhost" };
            let port = matches.value_of("port").unwrap().parse::<u32>().chain_err(|| "Expected integer")?;
            let addr = format!("{}:{}", host, port);
            let dir = std::path::Path::new(matches.value_of("directory").expect("required field missing")).canonicalize()?;
            fs_service(&addr, &dir)?;
        }
        ("serve", Some(matches)) => {
            let proxy_addr = matches.value_of("main-proxy").expect("required field missing");
            if proxy_addr.trim().is_empty() {
                bail!("Invalid `proxy` address")
            }
            let replace_host = matches.value_of("replace_host").map(str::to_owned).unwrap_or_else(|| {
                proxy_addr.split(":").nth(0).unwrap().to_owned()
            });
            let proxy_config = ProxyConfig { addr: proxy_addr.to_owned(), replace_host: Some(replace_host.into()) };

            let host = if matches.is_present("public") { "0.0.0.0" } else { "localhost" };
            let port = matches.value_of("port").unwrap().parse::<u32>().chain_err(|| "Expected integer")?;
            let addr = format!("{}:{}", host, port);

            let file_configs: Vec<FileConfig> = matches.values_of("exact-file").map(|vals| {
                vals.map(|val| {
                    let parts = val.split(",").collect::<Vec<_>>();
                    if parts.len() != 3 || parts.iter().any(|s| s.is_empty()) {
                        bail!("Invalid `--file` format. Expected `<exact-url>,<file-path>,<content-type>`")
                    }
                    Ok(FileConfig {
                        url: parts[0].to_owned(),
                        file_path: parts[1].to_owned(),
                        content_type: parts[2].to_owned(),
                    })
                }).collect::<Result<Vec<_>>>()
            }).unwrap_or_else(|| Ok(vec![]))?;

            let static_configs: Vec<StaticConfig> = matches.values_of("static-asset").map(|vals| {
                vals.map(|val| {
                    let parts = val.split(",").collect::<Vec<_>>();
                    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                        bail!("Invalid `--static` format. Expected `<url-prefix>,<path-root>`")
                    }
                    Ok(StaticConfig {
                        prefix: parts[0].to_owned(),
                        directory: parts[1].to_owned(),
                    })
                }).collect::<Result<Vec<_>>>()
            }).unwrap_or_else(|| Ok(vec![]))?;

            let subproxy_configs: Vec<SubProxyConfig> = matches.values_of("sub-proxy").map(|proxies| {
                proxies.map(|proxy| {
                    let parts = proxy.split(",").collect::<Vec<_>>();
                    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                        bail!("Invalid `--sub-proxy` format. Expected `<url-prefix>,<proxy-addr>`")
                    }
                    Ok(SubProxyConfig {
                        prefix: parts[0].to_owned(),
                        proxy: ProxyConfig {
                            addr: parts[1].to_owned(),
                            replace_host: Some(parts[1].split(":").nth(0).unwrap().to_owned().into()),
                        }
                    })
                }).collect::<Result<Vec<_>>>()
            }).unwrap_or_else(|| Ok(vec![]))?;

            proxy_service(&addr, proxy_config, subproxy_configs, static_configs, file_configs)?;
        }
        _ => eprintln!("proxy: see `--help`"),
    };
    Ok(())
}


quick_main!(run);


#[cfg(feature="update")]
fn update(matches: &ArgMatches) -> Result<()> {
    let mut builder = self_update::backends::github::Update::configure()?;

    builder.repo_owner("jaemk")
        .repo_name("proxy")
        .target(&self_update::get_target()?)
        .bin_name("proxy")
        .show_download_progress(true)
        .no_confirm(matches.is_present("no_confirm"))
        .current_version(crate_version!());

    if matches.is_present("quiet") {
        builder.show_output(false)
            .show_download_progress(false);
    }

    let status = builder.build()?.update()?;
    match status {
        self_update::Status::UpToDate(v) => {
            println!("Already up to date [v{}]!", v);
        }
        self_update::Status::Updated(v) => {
            println!("Updated to {}!", v);
        }
    }
    return Ok(());
}


#[cfg(not(feature="update"))]
fn update(_: &ArgMatches) -> Result<()> {
    bail!("This executable was not compiled with `self_update` features enabled via `--features update`")
}

