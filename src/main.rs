use clap::Parser;
use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server
};
use lazy_static::lazy_static;
use procfs::process::{all_processes, Status};
use procfs::ProcError;
use prometheus::{Encoder, IntGaugeVec, TextEncoder};
use prometheus::{opts, register_int_gauge_vec};
use prometheus::core::Collector;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use tokio::time::{sleep, Duration, Instant};
use users::{Users, UsersCache};

// declare all the prometheus metrics
lazy_static! {
    static ref USER_PROCESSES_GAUGE: IntGaugeVec = register_int_gauge_vec!(opts!(
        "node_user_processes",
        "The number of processes per user."),
        &["job", "hostgroup", "instance", "username"]
    )
    .unwrap();
    static ref USER_MEMORY_GAUGE: IntGaugeVec = register_int_gauge_vec!(opts!(
        "node_user_processes_rss",
        "The RSS on a node per user."),
        &["job", "hostgroup", "instance", "username"]
    )
    .unwrap();
    static ref USER_SWAP_GAUGE: IntGaugeVec = register_int_gauge_vec!(opts!(
        "node_user_processes_swap",
        "The swap on a node per user."),
        &["job", "hostgroup", "instance", "username"]
    )
    .unwrap();
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value_t = false)]
    oneshot: bool,

    #[arg(short, long)]
    port: Option<u16>,

    #[arg(long)]
    group: Option<String>,
    
    #[arg(long)]
    instance: Option<String>
}

fn get_all_procs() -> Result<Vec<Status>, ProcError> {
    // Get all processes
    Ok(all_processes()?
    .filter_map(|v| {
        v.and_then(|p| {
            Ok(p.status()?)
        })
        .ok()
    })
    .collect())
}

struct ProcEntry {
    count: i64,
    rss: i64,
    swap: i64
}

fn procs(usernames: &UsersCache, hostgroup: &str, instance: &str) {
    let processes = match get_all_procs() {
        Err(_) => {
            println!("Cannot get processes!");
            return;
        },
        Ok(procs) => procs
    };
    let mut user_procs = HashMap::new();

    for process in &processes {
        let user = usernames.get_user_by_uid(process.euid);
        let username = match &user {
            Some(x) => x.name().to_str().unwrap(),
            None => "unknown"
        };
        let entry = user_procs.entry(username.to_string()).or_insert(ProcEntry{count: 0, rss: 0, swap: 0});
        entry.count += 1;
        entry.rss += match process.vmrss {
            Some(x) => x as i64,
            None => 0
        } * 1000;
        entry.swap += match process.vmswap {
            Some(x) => x as i64,
            None => 0
        } * 1000;
    }

    let prev_metrics = USER_PROCESSES_GAUGE.collect();
    let mut prev_usernames = HashSet::with_capacity(prev_metrics.len());
    for m in &prev_metrics {
        for mm in m.get_metric() {
            match mm.get_label().last() {
                Some(x) => {
                    prev_usernames.insert(x.get_value());
                },
                None => { }
            }
        }
    }

    for (user, entry) in user_procs.into_iter() {
        let username = user.as_str();
        USER_PROCESSES_GAUGE.with_label_values(
            &["proc-mem-to-prom", hostgroup, instance, username]
        ).set(entry.count);
        USER_MEMORY_GAUGE.with_label_values(
            &["proc-mem-to-prom", hostgroup, instance, username]
        ).set(entry.rss);
        USER_SWAP_GAUGE.with_label_values(
            &["proc-mem-to-prom", hostgroup, instance, username]
        ).set(entry.swap);
        prev_usernames.remove(username);
    }

    for username in &prev_usernames {
        match USER_PROCESSES_GAUGE.remove_label_values(
            &["proc-mem-to-prom", hostgroup, instance, username]
        ) {
            _ => { }
        }
        match USER_MEMORY_GAUGE.remove_label_values(
            &["proc-mem-to-prom", hostgroup, instance, username]
        ) {
            _ => { }
        }
        match USER_SWAP_GAUGE.remove_label_values(
            &["proc-mem-to-prom", hostgroup, instance, username]
        ) {
            _ => { }
        }
    }
}

async fn serve_req(_req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    let mut buffer = Vec::<u8>::new();
    let encoder = TextEncoder::new();
    encoder.encode(&prometheus::gather(), &mut buffer).unwrap();

    let response = Response::builder()
        .status(200)
        .header(CONTENT_TYPE, encoder.format_type())
        .body(Body::from(buffer))
        .unwrap();
    Ok(response)
}


fn oneshot(group: &str, instance: &str) {
    let usernames = UsersCache::new();
    procs(&usernames, group, instance);
    
    // Print metrics for the default registry.
    let mut buffer = Vec::<u8>::new();
    let encoder = TextEncoder::new();
    encoder.encode(&prometheus::gather(), &mut buffer).unwrap();
    println!("## Default registry");
    println!("{}", String::from_utf8(buffer.clone()).unwrap());
}

async fn run_forever(group: &str, instance: &str) {
    let usernames = UsersCache::new();
    loop {
        let start = Instant::now();
        procs(&usernames, group, instance);
        sleep(Duration::from_secs(15) - start.elapsed()).await;
    }
}

#[tokio::main]
async fn main() {
    // get config
    let args = Args::parse();

    let env_port = env::var("PORT");
    let port = match args.port {
        Some(x) => x,
        None => match env_port {
            Ok(x) => match x.parse::<u16>() {
                Ok(x) => x,
                Err(_) => 0,
            },
            Err(_) => 0
        }
    };
    
    let env_group = env::var("GROUP");
    let group = match &args.group {
        Some(x) => x.as_str(),
        None => match &env_group {
            Ok(x) => x.as_str(),
            Err(_) => "test"
        }
    };

    let env_instance = env::var("INSTANCE");
    let instance = match &args.instance {
        Some(x) => x.as_str(),
        None => match &env_instance {
            Ok(x) => x.as_str(),
            Err(_) => "test"
        }
    };

    if args.oneshot {
        oneshot(&group, &instance);
        return;
    } else {
        // set up prometheus http reporter
        tokio::spawn(async move {
            let addr = ([0, 0, 0, 0], port).into();

            let serve_future = Server::bind(&addr).serve(make_service_fn(|_| async {
                Ok::<_, hyper::Error>(service_fn(serve_req))
            }));
            println!("Listening on http://{}", serve_future.local_addr());

            if let Err(err) = serve_future.await {
                eprintln!("server error: {}", err);
            }
        });
        // run prometheus
        run_forever(&group, &instance).await;
    }
}
