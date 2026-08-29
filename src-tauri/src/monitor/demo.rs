//! Demo Mode probe (spec §47): a scripted, looping "day" that exercises
//! focus, supporting work, a long distraction (to trigger the warning and
//! the intervention), and an idle stretch — without faking any downstream
//! logic. Everything after the probe is the real pipeline.

use std::time::Instant;

use super::{ActivityProbe, ProbeReading, RawSample};

struct Step {
    app: &'static str,
    process: &'static str,
    title: &'static str,
    domain: Option<&'static str>,
    duration_secs: u64,
    idle: bool,
}

const SCRIPT: &[Step] = &[
    Step {
        app: "Chrome",
        process: "chrome.exe",
        title: "Commercial Sales Playbook - Google Docs",
        domain: Some("docs.google.com"),
        duration_secs: 240,
        idle: false,
    },
    Step {
        app: "Chrome",
        process: "chrome.exe",
        title: "Inbox - Gmail",
        domain: Some("mail.google.com"),
        duration_secs: 45,
        idle: false,
    },
    Step {
        app: "Chrome",
        process: "chrome.exe",
        title: "Home / X",
        domain: Some("x.com"),
        duration_secs: 480, // long enough to cross warn (3m) and intervene (7m)
        idle: false,
    },
    Step {
        app: "Chrome",
        process: "chrome.exe",
        title: "Commercial Sales Playbook - Google Docs",
        domain: Some("docs.google.com"),
        duration_secs: 300,
        idle: false,
    },
    Step {
        app: "Idle",
        process: "",
        title: "",
        domain: None,
        duration_secs: 420, // idle threshold (180s) + a visible idle session
        idle: true,
    },
    Step {
        app: "Slack",
        process: "slack.exe",
        title: "#general - Slack",
        domain: None,
        duration_secs: 60,
        idle: false,
    },
    Step {
        app: "Chrome",
        process: "chrome.exe",
        title: "Commercial Sales Playbook - Google Docs",
        domain: Some("docs.google.com"),
        duration_secs: 300,
        idle: false,
    },
];

pub struct DemoProbe {
    step_idx: usize,
    step_started: Instant,
}

impl DemoProbe {
    pub fn new() -> Self {
        Self {
            step_idx: 0,
            step_started: Instant::now(),
        }
    }
}

impl ActivityProbe for DemoProbe {
    fn read(&mut self) -> ProbeReading {
        let mut elapsed = self.step_started.elapsed().as_secs();
        let mut step = &SCRIPT[self.step_idx];
        if elapsed >= step.duration_secs {
            self.step_idx = (self.step_idx + 1) % SCRIPT.len();
            self.step_started = Instant::now();
            step = &SCRIPT[self.step_idx];
            elapsed = 0;
        }
        ProbeReading::Sample(RawSample {
            app_name: step.app.into(),
            process_name: step.process.into(),
            window_title: step.title.into(),
            idle_seconds: if step.idle { elapsed } else { 0 },
            locked: false,
            browser_domain: step.domain.map(String::from),
            browser_title: step.domain.map(|_| step.title.to_string()),
        })
    }
}
