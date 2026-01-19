use std::env;
use std::process;
use std::time::SystemTime;

use domain::adapters::memory_repo::InMemoryRepo;
use domain::service::LinkService;
use domain::slug::Base62SlugGenerator;
use domain::{Clock, CoreError, NewLink, Slug, UserEmail};

struct StdClock;
impl Clock for StdClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

fn print_usage() {
    eprintln!(
        "{}\n\nUsage:\n  domain create <url> [--slug <custom>] [--user <email>]\n  domain resolve <slug>\n\nNotes:\n  - This demo CLI uses an in-memory repository; data is not persisted across runs.",
        domain::about()
    );
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1); // skip program name

    let Some(cmd) = args.next() else {
        print_usage();
        return Ok(());
    };

    // Construct a demo service with in-memory storage
    let repo = InMemoryRepo::new();
    let slugger = Base62SlugGenerator::new(1);
    let clock = StdClock;
    let svc = LinkService::new(repo, slugger, clock);

    match cmd.as_str() {
        "create" => {
            let Some(url) = args.next() else {
                return Err("missing <url> for create".into());
            };

            // Defaults for optional flags
            let mut custom_slug: Option<Slug> = None;
            let mut user = match UserEmail::new("dev@local") {
                Ok(e) => e,
                Err(_) => return Err("invalid default user".into()),
            };

            // Parse simple flags: --slug <val>, --user <email>
            let rest: Vec<String> = args.collect();
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--slug" => {
                        if i + 1 >= rest.len() {
                            return Err("--slug requires a value".into());
                        }
                        let val = rest[i + 1].clone();
                        match Slug::new(val) {
                            Ok(s) => custom_slug = Some(s),
                            Err(e) => return Err(format!("invalid custom slug: {}", e)),
                        }
                        i += 2;
                    }
                    "--user" => {
                        if i + 1 >= rest.len() {
                            return Err("--user requires an email".into());
                        }
                        let val = rest[i + 1].clone();
                        match UserEmail::new(val) {
                            Ok(e) => user = e,
                            Err(_) => return Err("invalid --user email".into()),
                        }
                        i += 2;
                    }
                    unk => {
                        return Err(format!("unknown argument: {}", unk));
                    }
                }
            }

            let input = NewLink {
                original_url: url,
                custom_slug,
                user_email: user,
            };
            match svc.create(input) {
                Ok(link) => {
                    println!("created: {} -> {}", link.slug.as_str(), link.original_url);
                    Ok(())
                }
                Err(e) => Err(format!("create failed: {}", e)),
            }
        }
        "resolve" => {
            let Some(slug_str) = args.next() else {
                return Err("missing <slug> for resolve".into());
            };
            let slug = match Slug::new(slug_str) {
                Ok(s) => s,
                Err(e) => return Err(format!("invalid slug: {}", e)),
            };
            match svc.resolve(&slug) {
                Ok(url) => {
                    println!("{}", url);
                    Ok(())
                }
                Err(CoreError::NotFound) => Err("not found".into()),
                Err(e) => Err(format!("resolve failed: {}", e)),
            }
        }
        _ => {
            print_usage();
            Ok(())
        }
    }
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("error: {}", msg);
        process::exit(1);
    }
}
