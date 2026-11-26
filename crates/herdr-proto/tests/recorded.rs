//! Real messages from an isolated herdr-core 0.1.22, recorded by `scripts/record_proto_fixtures.py`.

use std::collections::BTreeSet;

use herdr_proto::event::EventKind;
use herdr_proto::methods::*;
use herdr_proto::rpc::{Call, ErrorKind, Incoming};
use herdr_proto::{
    CloseReason, Decoder, EventLines, Frame, Hello, HelloParams, Kind, MIN_CORE, Notification,
    Reason, RpcError, TerminalSet,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const CALLS: &str = include_str!("fixtures/calls.jsonl");
const EVENTS: &str = include_str!("fixtures/events.jsonl");
const MISSED: &str = include_str!("fixtures/missed.jsonl");
const FRAMES: &[u8] = include_bytes!("fixtures/frames.bin");
const FRAME_LIST: &str = include_str!("fixtures/frames.jsonl");
const CLOSES: &str = include_str!("fixtures/close.jsonl");
const SCHEMA: &str = include_str!("fixtures/schema.json");

fn lines(text: &str) -> impl Iterator<Item = Value> + '_ {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
}

fn calls() -> Vec<Value> {
    lines(CALLS).collect()
}

/// Decodes a value as `T` and encodes it again: nothing may be lost or added.
fn round_trip<T: Serialize + DeserializeOwned>(value: &Value, what: &str) -> T {
    let typed: T =
        serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{what}: {e}\n{value}"));
    assert_eq!(
        &serde_json::to_value(&typed).unwrap(),
        value,
        "{what} changed in a round trip"
    );
    typed
}

/// Checks one recorded call: its params and its reply both survive a typed round trip.
fn check<C: Call + Serialize + DeserializeOwned>(call: &Value)
where
    C::Result: Serialize,
{
    let method = call["request"]["method"].as_str().unwrap();
    assert_eq!(method, C::METHOD);
    // The recording sends one malformed request on purpose; a typed one can't be malformed.
    if call["reply"]["error"]["data"]["kind"] != "invalid_params" {
        round_trip::<C>(&call["request"]["params"], &format!("{method} params"));
    }
    let bytes = serde_json::to_vec(&call["reply"]).unwrap();
    let Incoming::Response(response) = Incoming::parse(&bytes).unwrap() else {
        panic!("{method}: not a response")
    };
    assert_eq!(response.id, call["reply"]["id"].as_u64());
    match &response.outcome {
        Ok(result) => {
            round_trip::<C::Result>(result, &format!("{method} result"));
            response.into_result::<C>().unwrap();
        }
        Err(error) => {
            assert!(
                !matches!(error.kind, ErrorKind::Other(_)),
                "{method}: unknown kind {}",
                error.kind
            );
            round_trip::<RpcError>(&call["reply"]["error"], &format!("{method} error"));
        }
    }
}

#[test]
fn every_recorded_call_round_trips() {
    let mut seen = BTreeSet::new();
    for call in calls() {
        let method = call["request"]["method"].as_str().unwrap().to_owned();
        match method.as_str() {
            "hello" => check::<HelloParams>(&call),
            "server.status" => check::<ServerStatus>(&call),
            "templates.list" => check::<TemplatesList>(&call),
            "machine.list" => check::<MachineList>(&call),
            "terminal.list" => check::<TerminalList>(&call),
            "terminal.run" => check::<TerminalRun>(&call),
            "terminal.attach" => check::<TerminalAttach>(&call),
            "terminal.label" => check::<TerminalLabel>(&call),
            "terminal.keep" => check::<TerminalKeep>(&call),
            "terminal.get" => check::<TerminalGet>(&call),
            "terminal.read" => check::<TerminalRead>(&call),
            "terminal.keys" => check::<TerminalKeys>(&call),
            "terminal.send" => check::<TerminalSend>(&call),
            "terminal.interrupt" => check::<TerminalInterrupt>(&call),
            "terminal.put" => check::<TerminalPut>(&call),
            "terminal.wait" => check::<TerminalWait>(&call),
            "terminal.rm" => check::<TerminalRm>(&call),
            "terminal.kill" => check::<TerminalKill>(&call),
            "machine.exec" => check::<MachineExec>(&call),
            "file.write" => check::<FileWrite>(&call),
            "file.edit" => check::<FileEdit>(&call),
            "file.read" => check::<FileRead>(&call),
            "doc.get" => check::<DocGet>(&call),
            "doc.set" => check::<DocSet>(&call),
            "events.subscribe" => check::<EventsSubscribe>(&call),
            "nope.nope" => {
                let error: RpcError =
                    serde_json::from_value(call["reply"]["error"].clone()).unwrap();
                assert_eq!((error.code, error.kind), (-32601, ErrorKind::UnknownMethod));
            }
            other => panic!("no check for {other}"),
        }
        seen.insert(method);
    }
    assert_eq!(seen.len(), 26, "{seen:?}");
}

#[test]
fn recorded_errors_have_their_kinds() {
    let kinds: Vec<ErrorKind> = calls()
        .iter()
        .filter_map(|c| c["reply"].get("error"))
        .map(|e| serde_json::from_value::<RpcError>(e.clone()).unwrap().kind)
        .collect();
    assert_eq!(
        kinds,
        [
            ErrorKind::Conflict,
            ErrorKind::UnknownMethod,
            ErrorKind::NotFound,
            ErrorKind::InvalidParams,
            ErrorKind::MachineUnknown
        ]
    );
}

#[test]
fn the_recorded_hello_passes_the_check() {
    let calls = calls();
    let hello: Hello = serde_json::from_value(calls[0]["reply"]["result"].clone()).unwrap();
    let uses = [
        TerminalRun::METHOD,
        TerminalList::METHOD,
        TerminalKill::METHOD,
        TerminalLabel::METHOD,
        TerminalAttach::METHOD,
        TerminalKeep::METHOD,
        DocGet::METHOD,
        DocSet::METHOD,
        EventsSubscribe::METHOD,
    ];
    assert_eq!(hello.check(MIN_CORE, &uses), Ok(()));
    let caps = hello.capabilities();
    assert!(
        caps.framed
            && caps.attach
            && caps.events
            && caps.automatic_input
            && caps.resize_notifications
    );
}

/// The methods `herdr_proto::methods` types, beside the ones it leaves out on purpose.
#[test]
fn every_schema_method_is_typed_or_left_out_on_purpose() {
    let typed = [
        HelloParams::METHOD,
        ServerStatus::METHOD,
        RpcDiscover::METHOD,
        TerminalRun::METHOD,
        TemplatesList::METHOD,
        TerminalList::METHOD,
        TerminalGet::METHOD,
        TerminalKill::METHOD,
        TerminalRm::METHOD,
        TerminalKeep::METHOD,
        TerminalLabel::METHOD,
        TerminalRead::METHOD,
        TerminalAttach::METHOD,
        TerminalPut::METHOD,
        TerminalSend::METHOD,
        TerminalKeys::METHOD,
        TerminalInterrupt::METHOD,
        TerminalWait::METHOD,
        MachineList::METHOD,
        MachineAdd::METHOD,
        MachineConnect::METHOD,
        MachineRename::METHOD,
        MachineRm::METHOD,
        MachineExec::METHOD,
        FileRead::METHOD,
        FileWrite::METHOD,
        FileEdit::METHOD,
        DocGet::METHOD,
        DocSet::METHOD,
        EventsSubscribe::METHOD,
    ];
    let left_out = [
        "server.stop",
        "server.restart",
        "terminal.put_begin",
        "terminal.put_chunk",
        "terminal.put_finish",
        "terminal.put_abort",
    ];
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let methods: BTreeSet<&str> = schema["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    let covered: BTreeSet<&str> = typed.iter().chain(&left_out).copied().collect();
    assert_eq!(
        covered.len(),
        typed.len() + left_out.len(),
        "a method is listed twice"
    );
    assert_eq!(methods, covered);
}

/// Every param a typed struct sends is one the schema names.
#[test]
fn typed_params_use_the_schema_names() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let params_of = |method: &str| -> BTreeSet<String> {
        let m = schema["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == method)
            .unwrap();
        m["params"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let check = |method: &str, params: Value| {
        let sent: BTreeSet<String> = params.as_object().unwrap().keys().cloned().collect();
        let known = params_of(method);
        assert!(
            sent.is_subset(&known),
            "{method}: {:?} not in the schema",
            sent.difference(&known).collect::<Vec<_>>()
        );
    };
    let s = || Some("x".to_owned());
    let labels = || [("k".to_owned(), "v".to_owned())].into();
    check(
        "terminal.run",
        serde_json::to_value(TerminalRun {
            argv: vec!["sh".into()],
            line: s(),
            cwd: s(),
            env: labels(),
            labels: labels(),
            name: s(),
            on: s(),
            keep: true,
            size: Some(InitialSize { cols: 1, rows: 1 }),
            display: Some(Default::default()),
            parent: s(),
            no_parent: true,
            no_notify: true,
            outlive_parent: true,
            rm: true,
            wait: true,
            timeout: Some(1.0),
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.attach",
        serde_json::to_value(TerminalAttach {
            terminal: "t".into(),
            cols: 1,
            rows: 1,
            history: 1,
            display: Some(Default::default()),
            observe: true,
            resize_notifications: true,
            resumed: true,
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.send",
        serde_json::to_value(TerminalSend {
            terminal: "t".into(),
            text: "x".into(),
            force: true,
            only_if: Some(IdleOrWorking::Working),
            wait: true,
            timeout: Some(1.0),
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.wait",
        serde_json::to_value(TerminalWait {
            terminal: s(),
            children: Some(Children::Any),
            output: s(),
            timeout: Some(1.0),
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "events.subscribe",
        serde_json::to_value(EventsSubscribe {
            terminal: s(),
            children_of: s(),
            labels: labels(),
            docs: vec!["x".into()],
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "machine.add",
        serde_json::to_value(MachineAdd {
            target: s(),
            name: s(),
            session: s(),
            via: s(),
            temporary: true,
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "machine.exec",
        serde_json::to_value(MachineExec {
            argv: vec!["x".into()],
            line: s(),
            cwd: s(),
            env: labels(),
            input: s(),
            on: s(),
            timeout: Some(1.0),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "file.read",
        serde_json::to_value(FileRead {
            path: "p".into(),
            cwd: s(),
            offset: Some(1),
            limit: Some(1),
            on: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "file.write",
        serde_json::to_value(FileWrite {
            path: "p".into(),
            data: "d".into(),
            cwd: s(),
            on: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "file.edit",
        serde_json::to_value(FileEdit {
            path: "p".into(),
            edits: vec![Default::default()],
            cwd: s(),
            on: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.read",
        serde_json::to_value(TerminalRead {
            terminal: "t".into(),
            lines: 1,
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.keys",
        serde_json::to_value(TerminalKeys {
            terminal: "t".into(),
            keys: vec![],
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.interrupt",
        serde_json::to_value(TerminalInterrupt {
            terminal: "t".into(),
            timeout: Some(1.0),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.list",
        serde_json::to_value(TerminalList {
            all: true,
            labels: labels(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.get",
        serde_json::to_value(TerminalGet {
            terminal: "t".into(),
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.kill",
        serde_json::to_value(TerminalKill {
            terminal: "t".into(),
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.rm",
        serde_json::to_value(TerminalRm {
            terminal: "t".into(),
            caller: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.keep",
        serde_json::to_value(TerminalKeep {
            terminal: "t".into(),
            keep: true,
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.label",
        serde_json::to_value(TerminalLabel {
            terminal: "t".into(),
            labels: labels(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "terminal.put",
        serde_json::to_value(TerminalPut {
            terminal: "t".into(),
            name: "n".into(),
            data: "d".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "machine.connect",
        serde_json::to_value(MachineConnect {
            machine: "m".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "machine.rename",
        serde_json::to_value(MachineRename {
            machine: "m".into(),
            new_name: "n".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "machine.rm",
        serde_json::to_value(MachineRm {
            machine: "m".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "doc.get",
        serde_json::to_value(DocGet {
            name: "n".into(),
            on: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "doc.set",
        serde_json::to_value(DocSet {
            name: "n".into(),
            value: json!(1),
            version: 1,
            on: s(),
            ..Default::default()
        })
        .unwrap(),
    );
    check(
        "hello",
        serde_json::to_value(HelloParams {
            framed: true,
            generation: Some(1),
            core: Some(Default::default()),
            calls_you: s(),
            features: vec!["x".into()],
            ..Default::default()
        })
        .unwrap(),
    );
}

fn parse_events(text: &str) -> (Value, Vec<(Value, Notification)>) {
    let mut lines = lines(text);
    let subscribe = lines.next().unwrap()["subscribe"].clone();
    let notifications = lines
        .map(|line| {
            (
                line.clone(),
                Notification::parse(&serde_json::to_vec(&line).unwrap()).unwrap(),
            )
        })
        .collect();
    (subscribe, notifications)
}

#[test]
fn every_event_type_was_recorded_and_its_data_round_trips() {
    let (_, notifications) = parse_events(EVENTS);
    let mut kinds = BTreeSet::new();
    for (line, notification) in &notifications {
        match notification {
            Notification::Event(event) => {
                assert!(!matches!(event.kind(), EventKind::Other(_)), "{line}");
                assert_eq!(
                    event.data_value(),
                    line["params"]["data"],
                    "{} data changed in a round trip",
                    event.kind()
                );
                kinds.insert(event.kind().as_str().to_owned());
            }
            Notification::Heartbeat { seq } => {
                assert_eq!(Some(*seq), line["params"]["seq"].as_u64());
                kinds.insert("heartbeat".into());
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    let all = [
        "created",
        "lost",
        "restarted",
        "keep",
        "finished",
        "removed",
        "status",
        "turn",
        "title",
        "labels",
        "size",
        "foreground",
        "doc",
        "heartbeat",
    ];
    assert_eq!(kinds, all.iter().map(|k| (*k).to_owned()).collect());
}

#[test]
fn applying_the_recorded_events_gives_the_final_listing() {
    let (subscribe, notifications) = parse_events(EVENTS);
    let subscribed: SubscribeResult = serde_json::from_value(subscribe).unwrap();
    let mut set = TerminalSet::from(subscribed);
    let mut docs = Vec::new();
    for (_, notification) in notifications {
        let changes = set.apply(notification);
        docs.extend(
            changes
                .docs
                .into_iter()
                .map(|(key, version)| (key.name, version)),
        );
    }
    assert_eq!(
        docs,
        [("fixture-doc".to_owned(), 1), ("fixture-doc".to_owned(), 2)]
    );

    let listing = calls()
        .into_iter()
        .rev()
        .find(|c| c["request"]["method"] == "terminal.list")
        .unwrap();
    let listed: ListResult = serde_json::from_value(listing["reply"]["result"].clone()).unwrap();
    let got: Vec<Value> = set
        .iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    let want: Vec<Value> = listed
        .terminals
        .iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    assert_eq!(got, want);
}

#[test]
fn the_recorded_missed_resyncs() {
    let (subscribe, notifications) = parse_events(MISSED);
    let subscribed: SubscribeResult = serde_json::from_value(subscribe).unwrap();
    let mut set = TerminalSet::from(subscribed);
    let mut resyncs = 0;
    let mut before_missed = Vec::new();
    for (_, notification) in notifications {
        let replay = notification.clone();
        let changes = set.apply(notification);
        if changes.resynced {
            resyncs += 1;
            // Events from before the snapshot, delivered again, change nothing.
            for old in before_missed.drain(..) {
                assert!(set.apply(old).is_empty());
            }
        } else if resyncs == 0 {
            before_missed.push(replay);
        }
    }
    assert_eq!(resyncs, 1);
    let t1 = set.get("t1").unwrap();
    assert_eq!(
        t1.label("fixture.big"),
        None,
        "the last labels event removed it"
    );
    assert_eq!(t1.label("herdr.seen"), Some("1"));
}

/// Reads frames.bin in pieces of every size from 1 to 97 bytes.
fn decode_in_pieces(bytes: &[u8]) -> Vec<Frame> {
    let mut decoder = Decoder::new();
    let mut frames = Vec::new();
    let mut at = 0;
    for size in (1..=97).cycle() {
        if at >= bytes.len() {
            break;
        }
        let end = (at + size).min(bytes.len());
        decoder.push(&bytes[at..end]);
        at = end;
        while let Some(frame) = decoder.next_frame().unwrap() {
            frames.push(frame);
        }
    }
    assert!(decoder.is_between_frames());
    frames
}

#[test]
fn the_recorded_byte_stream_decodes_frame_by_frame() {
    let frames = decode_in_pieces(FRAMES);
    let listed: Vec<Value> = lines(FRAME_LIST).collect();
    let got: Vec<Value> = frames
        .iter()
        .map(|f| json!({"channel": f.channel, "kind": u8::from(f.kind), "len": f.payload.len()}))
        .collect();
    assert_eq!(got, listed);

    let mut whole = Decoder::new();
    whole.push(FRAMES);
    let at_once: Vec<Frame> = std::iter::from_fn(|| whole.next_frame().unwrap()).collect();
    assert_eq!(at_once, frames);

    let mut encoded = Vec::new();
    for frame in &frames {
        frame.encode_into(&mut encoded).unwrap();
    }
    assert_eq!(encoded, FRAMES, "frames encode back to the same bytes");
}

#[test]
fn the_recorded_channels_carry_replies_events_and_closes() {
    let frames = decode_in_pieces(FRAMES);
    let calls = calls();
    let events_channel = calls[1]["reply"]["result"]["channel"].as_u64().unwrap() as u32;

    let replies = frames
        .iter()
        .filter(|f| f.channel == 0)
        .map(|f| Incoming::parse(&f.payload).unwrap());
    assert!(replies.clone().all(|r| matches!(r, Incoming::Response(_))));
    assert!(replies.count() > 30);

    let mut events = EventLines::new();
    let mut notifications = Vec::new();
    for frame in frames
        .iter()
        .filter(|f| f.channel == events_channel && f.kind == Kind::Data)
    {
        events.push(&frame.payload);
        while let Some(n) = events.next_notification() {
            notifications.push(n.unwrap());
        }
    }
    let (_, recorded) = parse_events(EVENTS);
    assert!(!notifications.is_empty());
    for (got, (_, want)) in notifications.iter().zip(&recorded) {
        assert_eq!(got, want);
    }

    let closes: Vec<CloseReason> = frames.iter().filter_map(Frame::close_reason).collect();
    let reasons: Vec<Option<Reason>> = closes.iter().map(|c| c.reason.clone()).collect();
    assert_eq!(reasons, [Some(Reason::Signaled), Some(Reason::Killed)]);
}

#[test]
fn recorded_close_reasons_round_trip() {
    let reasons: Vec<CloseReason> = lines(CLOSES)
        .map(|line| round_trip::<CloseReason>(&line, "close reason"))
        .collect();
    assert_eq!(reasons[0].reason, Some(Reason::Signaled));
    assert_eq!(reasons[0].signal, Some(9));
    assert_eq!(reasons[1].reason, Some(Reason::Killed));
    assert!(reasons[1].finished_ms.is_some());
}
