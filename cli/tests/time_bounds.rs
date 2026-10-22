//! `messages` and `transcript` `--since`/`--until`: any RFC3339 offset or a bare date, compared
//! as an instant against the archive's stored `+00:00` timestamps; anything else is a usage error.

use assert_cmd::Command;
use serde_json::Value;

const CHAT: &str = "chat-range";

struct Archive {
    temp: tempfile::TempDir,
}

impl Archive {
    /// Five synthetic messages around 2026-01-01 UTC: the last minute of the day before, exactly
    /// midnight, and three in the morning. `p3` at 09:02:00 is the boundary row.
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let rows = [
            ("p0", "2025-12-31T23:59:00Z"),
            ("p1", "2026-01-01T00:00:00Z"),
            ("p2", "2026-01-01T09:00:00Z"),
            ("p3", "2026-01-01T09:02:00Z"),
            ("p4", "2026-01-01T09:05:00Z"),
        ];
        let lines: Vec<String> = rows
            .iter()
            .map(|(id, timestamp)| {
                serde_json::json!({
                    "account_hash": "acct-synthetic", "chat_id": CHAT,
                    "chat_name": "Synthetic Range", "chat_type": "group", "message_id": id,
                    "sender_id": "u1", "sender_nickname": "Synthetic", "timestamp": timestamp,
                    "text": format!("synthetic text {id}"), "message_type": "text",
                    "reply_to_message_id": null,
                })
                .to_string()
            })
            .collect();
        let fixture = temp.path().join("range.jsonl");
        std::fs::write(&fixture, lines.join("\n")).expect("fixture");
        let archive = Self { temp };
        archive
            .command(&["sync", "--source", "fixture", fixture.to_str().unwrap()])
            .assert()
            .success();
        archive
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::cargo_bin("katok").expect("katok binary");
        command
            .env("HOME", self.temp.path())
            .env_remove("KATOK_ARCHIVE_KEY")
            .arg("--data-dir")
            .arg(self.temp.path().join("data"))
            .args(args);
        command
    }

    fn messages(&self, range: &[&str]) -> Value {
        let mut args = vec!["messages", "--chat", CHAT, "--json"];
        args.extend_from_slice(range);
        let out = self.command(&args).assert().success().get_output().clone();
        serde_json::from_slice(&out.stdout).expect("messages json")
    }

    fn ids(&self, range: &[&str]) -> Vec<String> {
        self.messages(range)["messages"]
            .as_array()
            .expect("messages array")
            .iter()
            .map(|message| message["message_id"].as_str().unwrap().to_string())
            .collect()
    }

    /// How many messages `transcript --since <since>` exported.
    fn transcript_count(&self, since: &str) -> u64 {
        let out = self
            .command(&["transcript", "--chat", CHAT, "--since", since, "--json"])
            .assert()
            .success()
            .get_output()
            .clone();
        let report: Value = serde_json::from_slice(&out.stdout).expect("transcript json");
        report["messages"].as_u64().expect("message count")
    }
}

#[test]
fn an_offset_selects_the_same_rows_as_its_utc_equivalent() {
    let archive = Archive::new();
    let utc = archive.ids(&["--since", "2026-01-01T09:02:00Z"]);
    assert_eq!(utc, ["p3", "p4"]);
    assert_eq!(archive.ids(&["--since", "2026-01-01T18:02:00+09:00"]), utc);
    assert_eq!(archive.ids(&["--since", "2026-01-01T04:02:00-05:00"]), utc);

    let until_utc = archive.ids(&["--until", "2026-01-01T09:02:00Z"]);
    assert_eq!(until_utc, ["p0", "p1", "p2", "p3"]);
    assert_eq!(
        archive.ids(&["--until", "2026-01-01T18:02:00+09:00"]),
        until_utc
    );

    assert_eq!(
        archive.ids(&[
            "--since",
            "2026-01-01T09:00:00+09:00",
            "--until",
            "2026-01-01T18:00:00+09:00",
        ]),
        ["p1", "p2"],
        "a KST range is the UTC range nine hours earlier"
    );

    assert_eq!(archive.transcript_count("2026-01-01T18:02:00+09:00"), 2);
}

#[test]
fn the_row_at_the_bound_is_included_in_every_form_of_that_instant() {
    let archive = Archive::new();
    // What a caller gets back from katok and may pass in again as a cursor.
    let all = archive.messages(&[]);
    let stored = all["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["message_id"] == "p3")
        .expect("boundary row")["timestamp"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        stored, "2026-01-01T09:02:00+00:00",
        "the archive stores DateTime<Utc>::to_rfc3339(); the normalization targets this form"
    );

    for bound in [
        stored.as_str(),
        "2026-01-01T09:02:00Z",
        "2026-01-01T09:02:00+00:00",
        "2026-01-01T18:02:00+09:00",
    ] {
        assert_eq!(
            archive.ids(&["--since", bound]).first().map(String::as_str),
            Some("p3"),
            "--since {bound} must include the row stored at that instant"
        );
        assert_eq!(
            archive.ids(&["--until", bound]).last().map(String::as_str),
            Some("p3"),
            "--until {bound} must include the row stored at that instant"
        );
        assert_eq!(
            archive.transcript_count(bound),
            2,
            "transcript --since {bound}"
        );
    }
}

#[test]
fn a_bare_date_is_midnight_utc() {
    let archive = Archive::new();
    assert_eq!(
        archive.ids(&["--since", "2026-01-01"]),
        ["p1", "p2", "p3", "p4"]
    );
    assert_eq!(
        archive.ids(&["--until", "2026-01-01"]),
        ["p0", "p1"],
        "a bare --until is the start of that day, its midnight row included"
    );
    assert!(archive.ids(&["--since", "2026-01-02"]).is_empty());
    assert_eq!(archive.transcript_count("2026-01-01"), 4);
}

#[test]
fn the_output_echoes_the_bounds_as_given() {
    let archive = Archive::new();
    let output = archive.messages(&[
        "--since",
        "2026-01-01T18:02:00+09:00",
        "--until",
        "2026-01-01",
    ]);
    assert_eq!(output["since"], "2026-01-01T18:02:00+09:00");
    assert_eq!(output["until"], "2026-01-01");
    let unbounded = archive.messages(&[]);
    assert!(unbounded["since"].is_null() && unbounded["until"].is_null());
}

#[test]
fn an_unparseable_bound_is_a_usage_error() {
    let archive = Archive::new();
    for range in [
        ["--since", "soon"],
        ["--until", "2026-13-01"],
        ["--since", "2026-01-01T09:00:00"],
    ] {
        let mut args = vec!["messages", "--chat", CHAT];
        args.extend_from_slice(&range);
        archive.command(&args).assert().code(2);

        args.push("--json");
        let out = archive.command(&args).assert().code(2).get_output().clone();
        let envelope: Value = serde_json::from_slice(&out.stdout).expect("json error envelope");
        assert_eq!(envelope["ok"], false, "{range:?}");
        assert_eq!(envelope["error"]["code"], "invalid_arguments", "{range:?}");
    }
    archive
        .command(&["transcript", "--chat", CHAT, "--since", "soon", "--json"])
        .assert()
        .code(2);
}
