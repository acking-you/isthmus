//! A second Isthmus device in a terminal: try pairing and sending files
//! against the app on this computer or another, without a second app.
//!
//! ```text
//! cargo run -p isthmus-link --example peer -- "Test peer"
//! ```
//!
//! It announces itself on the network like the app, prints the code both
//! devices show when asked to pair, and takes commands on stdin: `list`,
//! `pair <n>`, `accept`, `code <digits>` (pairing with a version 1 device),
//! `send <n> <path>…`, `cancel`, `quit`. Its state lives in
//! the system's temp folder, apart from the app's; received files land in
//! `Received` there.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use isthmus_link::status::{Direction, PairRole, PairStage, TransferState};
use isthmus_link::{LinkOptions, LinkService, LinkStatus};

fn main() {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Isthmus test peer".to_owned());
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let root = std::env::temp_dir().join(format!("isthmus-peer-{slug}"));
    let data_dir = root.join("data");
    let receive_dir = root.join("Received");
    std::fs::create_dir_all(&data_dir).unwrap();

    let (tx, changes) = mpsc::channel();
    let service = Arc::new(
        LinkService::start(LinkOptions {
            data_dir,
            on_change: Some(Arc::new(move || {
                let _ = tx.send(());
            })),
            discovery: true,
            port: None,
            receive_dir: Some(receive_dir.clone()),
            loopback: false,
            protocol_version: isthmus_link::wire::PROTOCOL_VERSION,
            on_clipboard: Some(Arc::new(|_, text| {
                println!("* clipboard from a paired device: {text}");
            })),
        })
        .expect("file sharing starts"),
    );
    service.rename(&name).expect("the name is valid");
    let me = service.status().me;
    println!("{} ({}) listening on port {}", me.name, me.id, me.port);
    for path in &me.paths {
        println!(
            "  reachable at {} on {} ({:?})",
            path.address, path.interface, path.kind
        );
    }
    println!("received files go to {}", receive_dir.display());
    println!(
        "commands: list | pair <n> | accept | code <digits> | send <n> <path>... | cancel | quit"
    );

    let watcher = service.clone();
    std::thread::spawn(move || {
        let mut last = Seen::default();
        while changes.recv().is_ok() {
            // Fold a burst of hints into one look.
            while changes.try_recv().is_ok() {}
            last.report(&watcher.status());
        }
    });

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        let status = service.status();
        let peer = |index: Option<&&str>| {
            index
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| status.peers.get(n))
                .map(|peer| peer.id.clone())
        };
        let result = match words.as_slice() {
            ["list"] => {
                for (n, peer) in status.peers.iter().enumerate() {
                    let state = match (&peer.connection, peer.paired) {
                        (Some(c), _) => format!("connected over {:?}", c.kind),
                        (None, true) => "paired, offline".to_owned(),
                        (None, false) => format!("nearby on {:?}", peer.routes),
                    };
                    println!("  {n}: {} ({:?}) {state}", peer.name, peer.os);
                }
                Ok(())
            }
            ["pair", index] => match peer(Some(index)) {
                Some(id) => service.pair(&id).map_err(|e| e.to_string()),
                None => Err("no such device; try `list`".into()),
            },
            ["accept"] => service.accept_pairing().map_err(|e| e.to_string()),
            ["code", digits @ ..] => service
                .enter_code(&digits.concat())
                .map_err(|e| e.to_string()),
            ["send", index, paths @ ..] if !paths.is_empty() => match peer(Some(index)) {
                Some(id) => service
                    .send(&id, paths.iter().map(PathBuf::from).collect())
                    .map(|transfer| println!("  sending ({transfer})"))
                    .map_err(|e| e.to_string()),
                None => Err("no such device; try `list`".into()),
            },
            ["cancel"] => {
                service.cancel_pairing();
                Ok(())
            }
            ["quit"] | ["exit"] => break,
            [] => Ok(()),
            _ => Err(
                "commands: list | pair <n> | accept | code <digits> | send <n> <path>... | cancel | quit"
                    .into(),
            ),
        };
        if let Err(message) = result {
            println!("  {message}");
        }
    }
    service.shutdown();
}

/// What was last printed, so each change prints only what is new.
#[derive(Default)]
struct Seen {
    peers: HashMap<String, String>,
    pairing: Option<String>,
    transfers: HashMap<String, TransferState>,
}

impl Seen {
    fn report(&mut self, status: &LinkStatus) {
        for peer in &status.peers {
            let line = match (&peer.connection, peer.paired) {
                (Some(c), _) => format!("connected over {:?} ({})", c.kind, c.address),
                (None, true) => "paired, offline".to_owned(),
                (None, false) => format!("nearby on {:?}", peer.routes),
            };
            if self.peers.get(peer.id.as_str()) != Some(&line) {
                println!("* {}: {line}", peer.name);
                self.peers.insert(peer.id.to_string(), line);
            }
        }
        let pairing = status
            .pairing
            .as_ref()
            .map(|p| match (p.role, p.stage, &p.code) {
                (PairRole::Incoming, PairStage::Confirm, Some(code)) => format!(
                    "{} wants to pair; both show {} {}. Type `accept`, or `cancel` to decline",
                    p.peer_name,
                    &code[..3],
                    &code[3..]
                ),
                (PairRole::Outgoing, PairStage::Confirm, Some(code)) => format!(
                    "both devices show {} {}; waiting for {} to accept",
                    &code[..3],
                    &code[3..],
                    p.peer_name
                ),
                (PairRole::Incoming, PairStage::Code, Some(code)) => format!(
                    "{} wants to pair. Type this code there: {} {}",
                    p.peer_name,
                    &code[..3],
                    &code[3..]
                ),
                (PairRole::Outgoing, PairStage::Code, _) => format!(
                    "{} shows a code now. Enter it here: code <digits>",
                    p.peer_name
                ),
                (_, stage, _) => format!("pairing with {}: {stage:?} {:?}", p.peer_name, p.failure),
            });
        if pairing != self.pairing {
            if let Some(line) = &pairing {
                println!("* {line}");
            }
            self.pairing = pairing;
        }
        for transfer in &status.transfers {
            if self.transfers.get(&transfer.id) == Some(&transfer.state) {
                continue;
            }
            self.transfers.insert(transfer.id.clone(), transfer.state);
            if transfer.state.is_finished() {
                let way = match transfer.direction {
                    Direction::Send => format!("to {}", transfer.peer_name),
                    Direction::Receive => format!("from {}", transfer.peer_name),
                };
                println!(
                    "* {} {way}: {:?} {:?} ({} bytes over {:?}){}",
                    transfer.name,
                    transfer.state,
                    transfer.failure,
                    transfer.total_bytes,
                    transfer.path,
                    transfer
                        .saved_to
                        .as_ref()
                        .map(|p| format!(", saved to {p}"))
                        .unwrap_or_default()
                );
            }
        }
    }
}
