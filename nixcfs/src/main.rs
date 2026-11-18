//! nixcfs: serve the Nix store through composefs.

mod boot;
mod entries;
mod layout;
mod nar;
mod repo;
mod seal;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use composefs::{
    fsverity::{Algorithm, FsVerityHashValue, Sha256HashValue, Sha512HashValue},
    repository::{Repository, RepositoryConfig},
};
use rustix::fs::{CWD, Mode, OFlags, openat};

use layout::*;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Root directory the layout is relative to (/ normally, /sysroot in the initrd).
    #[arg(long, global = true, default_value = "/")]
    root: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create the repository (once, at install time).
    Init {
        #[arg(long, default_value = "fsverity-sha256-12")]
        algorithm: Algorithm,
    },
    /// Build a new image containing every valid store path; prints a JSON report.
    Seal {
        /// Physical directories holding unsealed store paths (default: auto).
        #[arg(long)]
        source: Vec<PathBuf>,
        /// Digest of the booted image (default: /run/nixcfs/image when --root is /).
        #[arg(long)]
        booted: Option<String>,
        #[arg(long)]
        no_nar_check: bool,
        #[arg(long, default_value_t = default_jobs())]
        jobs: usize,
        /// Only seal the closures of these store paths (testing/benchmarks).
        #[arg(long)]
        only_closure: Vec<PathBuf>,
    },
    /// Initrd: verify the image named on the kernel command line, scrub the
    /// upper layer and mount the store.
    Boot {
        #[arg(long)]
        digest: Option<String>,
        /// Mount point (default: <root>/nix/store).
        #[arg(long)]
        target: Option<PathBuf>,
    },
    /// Stamp the image digest on every boot entry on the ESP.
    Entries {
        #[arg(long, default_value = "/boot")]
        esp: PathBuf,
        /// Image (default: the latest seal).
        #[arg(long)]
        image: Option<String>,
    },
    /// Drop unreferenced images and objects.
    Gc {
        #[arg(long)]
        esp: Option<PathBuf>,
    },
    /// Seal, stamp entries, gc: what the bootloader install hook runs.
    UpdateBoot {
        #[arg(long, default_value = "/boot")]
        esp: PathBuf,
        #[arg(long)]
        no_nar_check: bool,
        #[arg(long, default_value_t = default_jobs())]
        jobs: usize,
    },
    /// Show the state of the store.
    Status,
}

fn default_jobs() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

fn algorithm(layout: &Layout) -> Result<Algorithm> {
    let fd = openat(CWD, layout.repo(), OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())
        .with_context(|| format!("opening {}", layout.repo().display()))?;
    composefs::repository::read_repo_algorithm(&fd)?.context("repository has no meta.json")
}

fn print<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn closure_names(paths: &[PathBuf], root: &Path) -> Result<HashSet<String>> {
    let out = std::process::Command::new("nix-store")
        .arg("--store")
        .arg(root)
        .arg("-qR")
        .args(paths)
        .output()
        .context("running nix-store -qR")?;
    if !out.status.success() {
        bail!("nix-store -qR failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8(out.stdout)?
        .lines()
        .filter_map(|l| l.strip_prefix("/nix/store/").map(str::to_owned))
        .collect())
}

fn run<ID: FsVerityHashValue>(cli: Cli) -> Result<()> {
    let layout = Layout::new(&cli.root);
    let rt = || tokio::runtime::Builder::new_multi_thread().enable_all().build();
    match cli.cmd {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Seal { source, booted, no_nar_check, jobs, only_closure } => {
            let only = if only_closure.is_empty() { None } else { Some(closure_names(&only_closure, &cli.root)?) };
            let rep = rt()?.block_on(seal::seal::<ID>(seal::SealOptions {
                layout,
                sources: source,
                booted,
                nar_check: !no_nar_check,
                jobs,
                only,
            }))?;
            print(&rep)
        }
        Cmd::Boot { digest, target } => {
            let digest = match digest {
                Some(d) => d,
                None => {
                    let cmdline = std::fs::read_to_string("/proc/cmdline")?;
                    boot::cmdline_digest(&cmdline).with_context(|| {
                        format!("no {CMDLINE_KEY}= on the kernel command line; refusing to mount an unverified store")
                    })?
                }
            };
            let target = target.unwrap_or_else(|| layout.store());
            let rep = boot::boot::<ID>(&layout, &digest, &target)?;
            std::fs::create_dir_all(RUN_DIR)?;
            std::fs::write(Path::new(RUN_DIR).join("image"), format!("{}\n", rep.image))?;
            std::fs::write(Path::new(RUN_DIR).join("boot-report.json"), serde_json::to_vec_pretty(&rep)?)?;
            print(&rep)
        }
        Cmd::Entries { esp, image } => {
            let image: ID = match image {
                Some(i) => repo::parse_digest(&i)?,
                None => repo::read_ref(&layout.repo(), LATEST_REF)?.context("nothing sealed yet")?,
            };
            print(&entries::update_entries(&layout, &esp, &image)?)
        }
        Cmd::Gc { esp } => print(&entries::gc::<ID>(&layout, esp.as_deref())?),
        Cmd::UpdateBoot { esp, no_nar_check, jobs } => {
            let rep = rt()?.block_on(seal::seal::<ID>(seal::SealOptions {
                layout: layout.clone(),
                sources: vec![],
                booted: None,
                nar_check: !no_nar_check,
                jobs,
                only: None,
            }))?;
            eprintln!(
                "nixcfs: sealed {} store paths ({} new, {} reused) into image {} in {:.1}s",
                rep.valid_paths, rep.scanned_paths, rep.reused_paths, rep.image, rep.seconds
            );
            let image: ID = repo::parse_digest(&rep.image)?;
            let e = entries::update_entries(&layout, &esp, &image)?;
            let g = entries::gc::<ID>(&layout, Some(&esp))?;
            eprintln!("nixcfs: gc removed {} objects ({} bytes)", g.objects_removed, g.bytes_freed);
            print(&serde_json::json!({"seal": rep, "entries": e, "gc": g}))
        }
        Cmd::Status => {
            let latest = repo::read_ref::<ID>(&layout.repo(), LATEST_REF)?.map(|d| d.to_hex());
            let booted = std::fs::read_to_string(Path::new(RUN_DIR).join("image")).ok().map(|s| s.trim().to_owned());
            let unsealed = std::fs::read_dir(layout.upper())
                .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| std::fs::symlink_metadata(e.path()).map(|m| !seal::is_whiteout(&m)).unwrap_or(false)).count())
                .unwrap_or(0);
            print(&serde_json::json!({
                "store_is_overlay": seal::store_is_overlay(&layout.store()),
                "booted_image": booted,
                "latest_image": latest,
                "unsealed_upper_entries": unsealed,
            }))
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    if let Cmd::Init { algorithm } = &cli.cmd {
        let layout = Layout::new(&cli.root);
        let path = layout.repo();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let cfg = RepositoryConfig::new(*algorithm);
        match algorithm {
            Algorithm::Sha256 { .. } => drop(Repository::<Sha256HashValue>::init_path(CWD, &path, cfg)?),
            Algorithm::Sha512 { .. } => drop(Repository::<Sha512HashValue>::init_path(CWD, &path, cfg)?),
        }
        std::fs::create_dir_all(layout.rw())?;
        println!("initialized {} ({algorithm})", path.display());
        return Ok(());
    }
    match algorithm(&Layout::new(&cli.root))? {
        Algorithm::Sha256 { .. } => run::<Sha256HashValue>(cli),
        Algorithm::Sha512 { .. } => run::<Sha512HashValue>(cli),
    }
}
