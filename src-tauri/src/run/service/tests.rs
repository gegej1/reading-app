//! The run service against a throwaway knowledge base and a **fake engine**:
//! a shell script (selected through `claudePath` / `codexPath`, like a user
//! would point the app at another binary) that records how it was called,
//! prints canned event lines and edits files the way a scenario needs. The
//! program is the same for every test (see `test_support::launcher`); what
//! it does comes from a scenario file of the test's own.
//!
//! The gate is TypeScript and runs in the webview; here a small stand-in
//! answers in its place (see `honest_gate`). The gate's own rules are tested
//! in `src/domain/gate.test.ts`; what is tested here is what the service does
//! with a report, and what it refuses whatever the report says.

use super::*;
use crate::kb::capture;
use crate::kb::test_support::{commit_count, git, init_repo, launcher, TempDir};
use crate::run::gate::GateDoc;
use crate::shell_env::{Origin, ShellEnv};
use std::sync::atomic::AtomicUsize;
use std::sync::OnceLock;

const CAPTURE_A: &str = "inbox/2026-10-12T09-00-00-mac.md";
const CAPTURE_B: &str = "inbox/2026-10-12T09-05-00-mac.md";
const ANSWER: &str = "inbox/裁决/2026-10-12T10-00-00-mac.md";
const RAW_OLD: &str = "raw/inbox/2026-10-11-收件箱.md";
const RAW_NEW: &str = "raw/inbox/新-收件箱.md";
/// The environment variables that tell the fake engines which scenario to play.
const CLAUDE_SCENARIO: &str = "DISE_FAKE_CLAUDE";
const CODEX_SCENARIO: &str = "DISE_FAKE_CODEX";
const REPORT: &str = "reports/新-入库报告.md";

/// What a well-behaved Claude Code prints at the end.
const RESULT: &str = r#"printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"{\"任务\": \"入库\", \"条数\": 2}","total_cost_usd":0.25,"num_turns":7,"duration_ms":1234,"permission_denials":[]}'"#;

/// Copies both captures into raw/, writes a report, extends the index.
const INGEST: &str = r#"mkdir -p raw/inbox reports
printf '# 收件箱\n\n## I-01\n周五做一次周回顾。\n\n## I-02\n清单越短越好。\n' > 'raw/inbox/新-收件箱.md'
printf '# 入库报告\n\n- 结果：2 条\n' > 'reports/新-入库报告.md'
printf -- '- [[新-入库报告]]\n' >> index.md
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"开始处理。"},{"type":"tool_use","name":"Write","input":{"file_path":"'"$PWD"'/raw/inbox/新-收件箱.md"}}]}}'
"#;

fn capture_text(body: &str) -> String {
    format!("---\n来源: mac\n时间: 2026-10-12T09:00:00+08:00\n---\n\n{body}\n")
}

fn issue(rule: &str, path: Option<&str>, message: String) -> GateIssue {
    GateIssue {
        rule: rule.to_string(),
        path: path.map(String::from),
        message,
    }
}

/// A stand-in for the TypeScript gate, enough for these tests: every queue
/// body must be somewhere in raw/, and what the engine and git reported
/// counts. It does not know the other rules.
fn honest_gate(input: &GateInput) -> GateReport {
    let queued: Vec<&str> = input.queue.iter().map(|file| file.path.as_str()).collect();
    let raw: Vec<&str> = input
        .changes
        .iter()
        .filter(|change| change.path.starts_with("raw/"))
        .filter_map(|change| change.after.as_deref())
        .chain(
            input
                .unchanged
                .iter()
                .filter(|doc| doc.path.starts_with("raw/"))
                .map(|doc| doc.content.as_str()),
        )
        .collect();
    let mut violations = Vec::new();
    let mut delete = Vec::new();
    let mut restore = Vec::new();
    for GateDoc { path, content } in &input.queue {
        let body = content.rsplit("---\n").next().unwrap_or_default().trim();
        if raw.iter().any(|text| text.contains(body)) {
            delete.push(path.clone());
            continue;
        }
        violations.push(issue(
            "queueVerbatim",
            Some(path),
            format!("{path} 的正文没有逐字出现在 raw/ 里"),
        ));
        if input.changes.iter().any(|change| &change.path == path) {
            restore.push(path.clone());
        }
    }
    if let Some(failure) = &input.engine_failure {
        violations.push(issue("engine", None, format!("AI 没有正常结束：{failure}")));
    }
    for commit in &input.foreign_commits {
        violations.push(issue(
            "engineCommits",
            None,
            format!("多了一个不是 App 做的提交 {}", commit.hash),
        ));
    }
    for path in &input.untracked_changed {
        violations.push(issue("untracked", Some(path), format!("{path} 被改动了")));
    }
    let ok = violations.is_empty();
    GateReport {
        ok,
        summary: if ok {
            "通过检查".to_string()
        } else {
            format!("没通过检查：{} 个问题", violations.len())
        },
        violations,
        warnings: Vec::new(),
        commit_paths: input
            .changes
            .iter()
            .map(|change| change.path.clone())
            .filter(|path| !queued.contains(&path.as_str()))
            .collect(),
        delete_queue_paths: delete,
        restore_queue_paths: restore,
        final_line: None,
    }
}

type GateFn = dyn Fn(&GateInput) -> Option<GateReport> + Send + Sync;

/// How many of these tests work at the same time. Each one starts dozens of
/// processes (git, the fake engine); all of them at once slow the machine
/// down enough to disturb tests elsewhere that measure short waits.
const WORLDS_AT_ONCE: usize = 3;
static WORLDS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

/// One of the [`WORLDS_AT_ONCE`] places, given back when dropped.
struct Turn;

impl Turn {
    fn take() -> Turn {
        let (count, freed) = &WORLDS;
        let mut busy = relock(count.lock());
        while *busy >= WORLDS_AT_ONCE {
            busy = relock(freed.wait(busy));
        }
        *busy += 1;
        Turn
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let (count, freed) = &WORLDS;
        *relock(count.lock()) -= 1;
        freed.notify_one();
    }
}

/// A knowledge base, a fake home folder, and a run service around them.
struct World {
    _turn: Turn,
    dir: TempDir,
    kb: PathBuf,
    lock: KbLock,
    env: ShellEnvState,
    service: RunService,
    notices: Arc<Mutex<Vec<Notice>>>,
    syncs: Arc<AtomicUsize>,
    sync_state: Arc<Mutex<SyncState>>,
    gate: Arc<Mutex<Arc<GateFn>>>,
    settings: Arc<Mutex<RunSettings>>,
}

impl World {
    fn new(tag: &str) -> Self {
        Self::with_timing(
            tag,
            RunTiming {
                gate_wait: Duration::from_secs(20),
                stop_grace: Duration::from_millis(300),
                lock_wait: Duration::from_secs(10),
            },
        )
    }

    fn with_timing(tag: &str, timing: RunTiming) -> Self {
        let turn = Turn::take();
        let dir = TempDir::new(tag);
        let kb = dir.mkdir("kb");
        init_repo(&kb);
        let write = |rel: &str, content: &str| {
            dir.write(&format!("kb/{rel}"), content);
        };
        write(".gitignore", ".DS_Store\n");
        write("AGENTS.md", "# 规则\n\n（测试用的规则文件）\n");
        write("README.md", "# 说明\n");
        write("index.md", "# 总目录\n");
        write("log.md", "# 操作日志\n");
        write("self/底色.md", "### D01 示例 ✅\n> 原话\n");
        write(RAW_OLD, "# 2026-10-11 收件箱\n\n## I-01\n第一条。\n");
        write("wiki/概念/小实验.md", "# 小实验\n");
        write("inbox/.gitkeep", "");
        write("inbox/裁决/.gitkeep", "");
        write(CAPTURE_A, &capture_text("周五做一次周回顾。"));
        write(CAPTURE_B, &capture_text("清单越短越好。"));
        write(ANSWER, "---\n对象: Q-004\n---\n\n先不统一。\n");
        git(&kb, &["add", "--all"]);
        git(&kb, &["commit", "-q", "-m", "init"]);
        // The user's own file, never added to git.
        write("草稿.md", "我自己的草稿\n");

        // What the engines would find in the user's home folder.
        dir.write(
            "home/.claude/settings.json",
            r#"{"apiKeyHelper": "print-fake-key", "env": {"ANTHROPIC_BASE_URL": "https://gateway.example.invalid", "API_TIMEOUT_MS": "600000", "UNRELATED_SECRET": "must-stay-behind"}, "permissions": {"allow": ["Bash(*)"], "defaultMode": "bypassPermissions"}, "hooks": {"Stop": []}}"#,
        );
        dir.write(
            "home/.codex/config.toml",
            "model = \"x\"\n\n[mcp_servers.notion]\nurl = \"https://example.invalid/mcp\"\n\n[mcp_servers.chrome-devtools]\ncommand = \"npx\"\n",
        );
        let scenario = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        let env = ShellEnvState::ready(ShellEnv {
            env: ChildEnv::from_vars([
                ("PATH".to_string(), std::env::var("PATH").unwrap_or_default()),
                (
                    "HOME".to_string(),
                    dir.path().join("home").to_string_lossy().into_owned(),
                ),
                // What the fake engines of this world do.
                (CLAUDE_SCENARIO.to_string(), scenario("fake-claude.sh")),
                (CODEX_SCENARIO.to_string(), scenario("fake-codex.sh")),
            ]),
            origin: Origin::LoginShell,
            took: Duration::ZERO,
        });

        let lock = KbLock::default();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let syncs = Arc::new(AtomicUsize::new(0));
        let sync_state = Arc::new(Mutex::new(SyncState::Synced));
        let gate: Arc<Mutex<Arc<GateFn>>> =
            Arc::new(Mutex::new(Arc::new(|input| Some(honest_gate(input)))));
        let settings = Arc::new(Mutex::new(RunSettings {
            budget_usd: 1.5,
            timeout: Duration::from_secs(60),
            ..RunSettings::default()
        }));
        // The hook needs the service it belongs to: filled in right below.
        let handle: Arc<OnceLock<RunService>> = Arc::new(OnceLock::new());

        let service = RunService::new(
            lock.clone(),
            env.clone(),
            RunDirs {
                runs: dir.path().join("data/runs"),
                temp: dir.path().join("tmp/dise"),
            },
            timing,
            Hooks {
                root: Box::new({
                    let kb = kb.clone();
                    move || Ok(kb.clone())
                }),
                settings: Box::new({
                    let settings = Arc::clone(&settings);
                    move || (relock(settings.lock()).clone(), Vec::new())
                }),
                sync: Box::new({
                    let syncs = Arc::clone(&syncs);
                    let state = Arc::clone(&sync_state);
                    move || {
                        syncs.fetch_add(1, Ordering::SeqCst);
                        SyncReport::stopped(*relock(state.lock()), None)
                    }
                }),
                notify: Box::new({
                    let notices = Arc::clone(&notices);
                    let gate = Arc::clone(&gate);
                    let handle = Arc::clone(&handle);
                    move |notice| {
                        // Plays the main webview: answers every check it is told about.
                        if let (Notice::Status(status), Some(service)) = (&notice, handle.get()) {
                            if let (Phase::Checking, Some(run)) = (status.phase, &status.run) {
                                if let Ok(input) = service.gate_input(&run.id, status.gate_attempt)
                                {
                                    let answer: Arc<GateFn> = relock(gate.lock()).clone();
                                    if let Some(report) = answer(&input) {
                                        let _ = service.gate_report(
                                            &run.id,
                                            status.gate_attempt,
                                            report,
                                        );
                                    }
                                }
                            }
                        }
                        relock(notices.lock()).push(notice);
                    }
                }),
            },
        );
        let _ = handle.set(service.clone());
        World {
            _turn: turn,
            dir,
            kb,
            lock,
            env,
            service,
            notices,
            syncs,
            sync_state,
            gate,
            settings,
        }
    }

    fn record_dir(&self) -> PathBuf {
        self.dir.path().join("record")
    }

    /// What the fake engine wrote down about its call, or `None` when it never ran.
    fn recorded(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.record_dir().join(name)).ok()
    }

    fn engine_calls(&self) -> usize {
        self.recorded("calls").map_or(0, |calls| calls.lines().count())
    }

    /// Installs a fake `claude` that does `body` after its init line.
    fn claude(&self, body: &str) {
        let script = format!(
            r#"record='{record}'
mkdir -p "$record"
echo run >> "$record/calls"
printf '%s\n' "$@" > "$record/args"
pwd > "$record/cwd"
env > "$record/env"
prev=''
for arg in "$@"; do
  if [ "$prev" = "--settings" ]; then
    echo "$arg" > "$record/settings-path"
    (stat -f '%Lp' "$arg" 2>/dev/null || stat -c '%a' "$arg") > "$record/settings-mode"
    cat "$arg" > "$record/settings-content"
  fi
  prev="$arg"
done
printf '%s\n' '{{"type":"system","subtype":"init","model":"fake-model","apiKeySource":"apiKeyHelper","claude_code_version":"0.0.0"}}'
{body}
"#,
            record = self.record_dir().display(),
        );
        self.dir.write("fake-claude.sh", &script);
        relock(self.settings.lock()).claude_path = Some(
            launcher("fake-claude", CLAUDE_SCENARIO)
                .to_string_lossy()
                .into_owned(),
        );
    }

    /// Installs a fake `codex` that does `body` and then ends its turn.
    fn codex(&self, body: &str) {
        let script = format!(
            r#"record='{record}'
mkdir -p "$record"
echo run >> "$record/calls"
printf '%s\n' "$@" > "$record/args"
cat > "$record/stdin"
out=''
prev=''
for arg in "$@"; do
  if [ "$prev" = "-o" ]; then out="$arg"; fi
  prev="$arg"
done
printf '%s\n' '{{"type":"thread.started","thread_id":"t"}}'
{body}
printf '%s' '{{"任务": "入库", "条数": 2}}' > "$out"
printf '%s\n' '{{"type":"turn.completed","usage":{{"input_tokens":10,"output_tokens":5}}}}'
"#,
            record = self.record_dir().display(),
        );
        self.dir.write("fake-codex.sh", &script);
        let mut settings = relock(self.settings.lock());
        settings.codex_path = Some(
            launcher("fake-codex", CODEX_SCENARIO)
                .to_string_lossy()
                .into_owned(),
        );
        settings.engine = EngineKind::Codex;
    }

    fn set_gate(&self, gate: impl Fn(&GateInput) -> Option<GateReport> + Send + Sync + 'static) {
        *relock(self.gate.lock()) = Arc::new(gate);
    }

    fn start(&self, task: Task) -> String {
        let status = self
            .service
            .start(StartRequest {
                task,
                overrides: Overrides::default(),
                trigger: "ui",
            })
            .expect("the run starts");
        status.run.expect("a run").id
    }

    fn wait(&self, what: &str, done: impl Fn(&RunStatus) -> bool) -> RunStatus {
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            let status = self.service.status();
            if done(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "waited too long for {what}: {status:#?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits until the run `id` has ended (and said so) and returns its record.
    fn finished(&self, id: &str) -> RunRecord {
        let ended = |status: &RunStatus| {
            status.phase == Phase::Idle
                && status.run.as_ref().is_some_and(|run| run.id == id)
                // The last thing a run does is to say that it is over.
                && matches!(
                    relock(self.notices.lock()).last(),
                    Some(Notice::Ended { .. })
                )
        };
        self.wait("the run to end", ended).run.unwrap()
    }

    /// Waits until the run `id` waits for a decision.
    fn undecided(&self, id: &str) -> RunRecord {
        let waiting = |status: &RunStatus| {
            status.phase == Phase::NeedsDecision
                && status.run.as_ref().is_some_and(|run| run.id == id)
        };
        self.wait("a decision to be needed", waiting).run.unwrap()
    }

    /// Waits until the run has shown a step with this text.
    fn step_shown(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(40);
        while !self
            .service
            .steps()
            .iter()
            .any(|event| event.step.text.as_deref() == Some(text))
        {
            assert!(Instant::now() < deadline, "the step {text:?} never came");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.kb.join(rel)).unwrap_or_else(|error| panic!("{rel}: {error}"))
    }

    fn exists(&self, rel: &str) -> bool {
        self.kb.join(rel).exists()
    }

    fn status_lines(&self) -> String {
        git(
            &self.kb,
            &["-c", "core.quotePath=false", "status", "--porcelain"],
        )
    }

    fn head_files(&self) -> Vec<String> {
        let listed = git(
            &self.kb,
            &[
                "-c",
                "core.quotePath=false",
                "show",
                "--name-status",
                "--format=",
                "HEAD",
            ],
        );
        let mut files: Vec<String> = listed.lines().map(|line| line.replace('\t', " ")).collect();
        files.sort();
        files
    }

    fn head_subject(&self) -> String {
        git(&self.kb, &["log", "-1", "--format=%s"])
    }

    fn ended_notices(&self) -> Vec<bool> {
        relock(self.notices.lock())
            .iter()
            .filter_map(|notice| match notice {
                Notice::Ended { files_changed } => Some(*files_changed),
                _ => None,
            })
            .collect()
    }

    fn rules(record: &RunRecord) -> Vec<String> {
        record
            .gate
            .as_ref()
            .map(|report| {
                report
                    .violations
                    .iter()
                    .map(|violation| violation.rule.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The auth files left in the private temp folder.
    fn auth_files(&self) -> Vec<String> {
        fs::read_dir(self.dir.path().join("tmp/dise"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[test]
fn a_good_run_is_committed_as_exactly_the_approved_paths() {
    let world = World::new("run-happy");
    world.claude(&format!("{INGEST}{RESULT}"));
    let commits = commit_count(&world.kb);

    let id = world.start(Task::Ingest);
    let record = world.finished(&id);

    assert_eq!(record.outcome, Some(Outcome::Committed), "{record:#?}");
    assert_eq!(record.queue, [CAPTURE_A, CAPTURE_B]);
    assert_eq!(record.engine, EngineKind::Claude);
    assert_eq!(record.trigger, "ui");
    // One commit: the approved files and the two processed captures, nothing else.
    assert_eq!(commit_count(&world.kb), commits + 1);
    assert_eq!(world.head_subject(), "入库：2 条（Claude）");
    assert_eq!(
        world.head_files(),
        [
            format!("A {RAW_NEW}"),
            format!("A {REPORT}"),
            format!("D {CAPTURE_A}"),
            format!("D {CAPTURE_B}"),
            "M index.md".to_string(),
        ]
    );
    assert_eq!(record.commit, Some(git(&world.kb, &["rev-parse", "HEAD"])));
    assert!(!world.exists(CAPTURE_A) && !world.exists(CAPTURE_B));
    assert_eq!(world.status_lines(), "?? 草稿.md", "the user's file is untouched");
    assert_eq!(world.read("草稿.md"), "我自己的草稿\n");

    // A sync came first, and another one is asked for once the commit is made.
    assert_eq!(world.syncs.load(Ordering::SeqCst), 1);
    assert_eq!(world.ended_notices(), [true]);
    // Syncs and edits are possible again.
    assert_eq!(world.lock.held_by(), None);
    assert!(world.lock.try_acquire("同步").is_ok());

    // What the engine reported is in the record, and in the run's folder.
    let engine = record.engine_report.expect("an engine report");
    assert!(engine.ok);
    assert_eq!(engine.stream.cost_usd, Some(0.25));
    assert_eq!(engine.stream.turns, Some(7));
    assert_eq!(engine.stream.model.as_deref(), Some("fake-model"));
    assert_eq!(engine.exit.code, Some(0));
    assert!(record.gate.as_ref().is_some_and(|report| report.ok));
    let folder = world.dir.path().join("data/runs").join(&id);
    let events = fs::read_to_string(folder.join(record::EVENTS_FILE)).unwrap();
    assert_eq!(events.lines().count(), 3, "{events}");
    assert!(events.lines().next().unwrap().contains("\"subtype\":\"init\""));
    assert_eq!(record::load(&folder).unwrap().outcome, Some(Outcome::Committed));
    assert_eq!(world.service.history()[0].id, id);

    // The steps a window would have shown.
    let steps = world.service.steps();
    let shown: Vec<String> = steps
        .iter()
        .map(|event| {
            format!(
                "{}{}{}",
                event.step.verb.as_deref().unwrap_or(""),
                event.step.target.as_deref().map(|target| format!(" {target}")).unwrap_or_default(),
                event.step.text.as_deref().unwrap_or("")
            )
        })
        .collect();
    assert!(shown.contains(&format!("写 {RAW_NEW}")), "{shown:#?}");
    assert!(shown.contains(&"开始处理。".to_string()), "{shown:#?}");
    assert!(shown.iter().any(|step| step.contains("模型 fake-model")), "{shown:#?}");
    assert!(steps.windows(2).all(|pair| pair[0].seq + 1 == pair[1].seq));
    assert_eq!(world.service.status().phase, Phase::Idle);
}

#[test]
fn claude_is_called_restricted_in_the_knowledge_base_with_a_private_login_file() {
    let world = World::new("run-claude-call");
    world.claude(&format!("{INGEST}{RESULT}"));
    let id = world.start(Task::Ingest);
    world.finished(&id);

    assert_eq!(world.recorded("cwd").unwrap().trim(), world.kb.to_string_lossy());
    let args: Vec<String> = world
        .recorded("args")
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(args[0], "-p");
    // The prompt is one argument; its lines follow in the record.
    assert_eq!(args[1], "你在我的个人决策底色知识库里工作。本次任务：入库。");
    let rest = args.join("\n");
    assert!(rest.contains(&format!("- {CAPTURE_A}\n- {CAPTURE_B}\n")), "{rest}");
    assert!(rest.contains(&format!("今天是 {}。", capture::LocalTime::now().date())));
    for expected in [
        "--restricted",
        "--tools\nRead,Write,Edit,Glob,Grep",
        "--permission-mode\nacceptEdits",
        "--permission-prompts\nnone",
        "--strict-mcp-config",
        "--mcp-config\n{\"mcpServers\":{}}",
        "--max-budget-usd\n1.5",
        "--output-format\nstream-json",
        "--verbose",
    ] {
        assert!(rest.contains(expected), "missing {expected:?} in {rest}");
    }
    assert!(rest.ends_with(&format!(
        "--append-system-prompt-file\n{}",
        world.kb.join("AGENTS.md").display()
    )));
    assert!(!rest.contains("--model"), "no model was asked for");

    // The login file: private, in the private folder, only the login in it …
    assert_eq!(world.recorded("settings-mode").unwrap().trim(), "600");
    let auth_path = PathBuf::from(world.recorded("settings-path").unwrap().trim());
    assert_eq!(auth_path.parent().unwrap(), world.dir.path().join("tmp/dise"));
    let auth: serde_json::Value =
        serde_json::from_str(&world.recorded("settings-content").unwrap()).unwrap();
    assert_eq!(
        auth,
        serde_json::json!({
            "apiKeyHelper": "print-fake-key",
            "env": {
                "ANTHROPIC_BASE_URL": "https://gateway.example.invalid",
                "API_TIMEOUT_MS": "600000"
            }
        })
    );
    // … and gone as soon as the engine is.
    assert!(!auth_path.exists());
    assert!(world.auth_files().is_empty());
    // The engine ran with the login-shell environment, not the app's own.
    let env = world.recorded("env").unwrap();
    assert!(env.contains(&format!("HOME={}", world.dir.path().join("home").display())));
    assert!(!env.contains("CARGO"), "{env}");
}

#[test]
fn a_request_can_pick_the_model() {
    let world = World::new("run-model");
    world.claude(&format!("{INGEST}{RESULT}"));
    let status = world
        .service
        .start(StartRequest {
            task: Task::Ingest,
            overrides: Overrides {
                engine: None,
                model: Some("sonnet".into()),
            },
            trigger: "command line",
        })
        .unwrap();
    let record = world.finished(&status.run.unwrap().id);
    assert_eq!(record.model.as_deref(), Some("sonnet"));
    assert_eq!(record.trigger, "command line");
    assert!(world.recorded("args").unwrap().contains("--model\nsonnet\n"));
}

#[test]
fn an_empty_queue_never_starts_the_engine() {
    let world = World::new("run-empty");
    world.claude(&format!("{INGEST}{RESULT}"));
    git(&world.kb, &["rm", "-q", "--", CAPTURE_A, CAPTURE_B]);
    git(&world.kb, &["commit", "-q", "-m", "empty the inbox"]);
    let commits = commit_count(&world.kb);

    let id = world.start(Task::Ingest);
    let record = world.finished(&id);
    assert_eq!(record.outcome, Some(Outcome::NothingToDo));
    assert_eq!(
        record.message.as_deref(),
        Some("收件箱是空的，没有要入库的随手记")
    );
    assert_eq!(world.engine_calls(), 0, "the engine costs money");
    assert_eq!(record.engine_report, None);
    // It synced first: a capture from another device would have been in the queue.
    assert_eq!(world.syncs.load(Ordering::SeqCst), 1);
    assert_eq!(commit_count(&world.kb), commits);
    assert_eq!(world.lock.held_by(), None);
    assert_eq!(world.ended_notices(), [false]);
    // The answers in inbox/裁决/ are another queue.
    assert!(world.exists(ANSWER));
}

#[test]
fn a_run_is_refused_when_the_knowledge_base_is_not_ready() {
    let world = World::new("run-refused");
    world.claude(&format!("{INGEST}{RESULT}"));

    // A tracked file with changes nobody committed.
    world.dir.write("kb/log.md", "# 操作日志\n改了没提交\n");
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Refused));
    assert!(record.message.as_deref().unwrap().contains("log.md"), "{record:#?}");
    git(&world.kb, &["checkout", "-q", "--", "log.md"]);

    // A sync that needs a person first.
    *relock(world.sync_state.lock()) = SyncState::Conflict;
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Refused));
    assert!(record.message.as_deref().unwrap().contains("同步没有成功"));
    *relock(world.sync_state.lock()) = SyncState::Offline;

    // No rules: the engine would not know what to do.
    git(&world.kb, &["rm", "-q", "--", "AGENTS.md"]);
    git(&world.kb, &["commit", "-q", "-m", "no rules"]);
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Refused));
    assert!(record.message.as_deref().unwrap().contains("AGENTS.md"));

    assert_eq!(world.engine_calls(), 0);
    assert_eq!(world.lock.held_by(), None);
    assert!(world.exists(CAPTURE_A));
}

#[test]
fn being_offline_does_not_stop_a_run() {
    let world = World::new("run-offline");
    world.claude(&format!("{INGEST}{RESULT}"));
    *relock(world.sync_state.lock()) = SyncState::Offline;
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Committed));
}

#[test]
fn a_change_outside_the_allowed_folders_is_never_committed() {
    let world = World::new("run-outside");
    world.claude(&format!(
        "{INGEST}printf '改了不该改的文件\\n' >> README.md\n{RESULT}"
    ));
    // Even a gate that waves everything through does not get it committed.
    world.set_gate(|input| {
        let mut report = honest_gate(input);
        assert!(report.ok, "the stand-in does not know the path rule");
        report.summary = "通过检查".into();
        Some(report)
    });
    let commits = commit_count(&world.kb);

    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);

    assert_eq!(World::rules(&record), ["paths"]);
    let report = record.gate.as_ref().unwrap();
    assert!(!report.ok);
    assert!(report.violations[0].message.contains("README.md"));
    assert_eq!(commit_count(&world.kb), commits, "nothing was committed");
    // Everything is still where the engine left it, and the queue is intact.
    assert!(world.read("README.md").contains("改了不该改的文件"));
    assert!(world.exists(RAW_NEW) && world.exists(CAPTURE_A) && world.exists(CAPTURE_B));
    // No sync or edit can pick the changes up while they wait …
    let busy = world.lock.try_acquire("同步").err().expect("the job is still there");
    assert_eq!(busy.operation, "等你处理入库的结果");
    assert!(world.ended_notices().is_empty());
    // … but a quick capture still works.
    assert!(world
        .lock
        .acquire_beside_job("写入", Duration::from_secs(1))
        .is_ok());
    // And no second run starts on top of it.
    let refused = world
        .service
        .start(StartRequest {
            task: Task::LandRulings,
            overrides: Overrides::default(),
            trigger: "ui",
        })
        .unwrap_err();
    assert!(refused.contains("还在等你处理"), "{refused}");
}

#[test]
fn a_raw_file_rewritten_in_the_middle_is_never_committed() {
    let world = World::new("run-raw");
    world.claude(&format!(
        "{INGEST}printf '# 2026-10-11 收件箱\\n\\n## I-01\\n改过的第一条。\\n' > '{RAW_OLD}'\n{RESULT}"
    ));
    let commits = commit_count(&world.kb);
    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);
    assert_eq!(World::rules(&record), ["rawAppendOnly"]);
    assert!(record.gate.unwrap().violations[0].message.contains(RAW_OLD));
    assert_eq!(commit_count(&world.kb), commits);
}

#[test]
fn appending_to_a_raw_file_is_fine() {
    let world = World::new("run-raw-append");
    world.claude(&format!(
        "{INGEST}printf '\\n## I-02\\n追加的一条。\\n' >> '{RAW_OLD}'\n{RESULT}"
    ));
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Committed), "{record:#?}");
    assert!(world.head_files().contains(&format!("M {RAW_OLD}")));
}

/// Copies only the first capture into raw/, and removes both queue files
/// (which Codex could do; Claude Code cannot delete).
const HALF_INGEST: &str = r#"mkdir -p raw/inbox reports
printf '# 收件箱\n\n## I-01\n周五做一次周回顾。\n' > 'raw/inbox/新-收件箱.md'
printf '# 入库报告\n' > 'reports/新-入库报告.md'
rm 'inbox/2026-10-12T09-00-00-mac.md' 'inbox/2026-10-12T09-05-00-mac.md'
"#;

#[test]
fn a_capture_that_was_not_copied_stays_and_keeping_anyway_is_recorded() {
    let world = World::new("run-not-copied");
    world.claude(&format!("{HALF_INGEST}{RESULT}"));
    let commits = commit_count(&world.kb);

    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);
    assert_eq!(World::rules(&record), ["queueVerbatim"]);
    let report = record.gate.as_ref().unwrap();
    assert_eq!(report.violations[0].path.as_deref(), Some(CAPTURE_B));
    assert_eq!(report.delete_queue_paths, [CAPTURE_A]);
    assert_eq!(report.restore_queue_paths, [CAPTURE_B]);
    assert_eq!(commit_count(&world.kb), commits);
    assert!(record.kept_despite.is_empty());

    // The user looked at it and wants it anyway.
    let status = world.service.keep(&id).unwrap();
    assert_eq!(status.phase, Phase::Idle);
    let kept = world.finished(&id);
    assert_eq!(kept.outcome, Some(Outcome::Kept));
    assert_eq!(kept.kept_despite.len(), 1);
    assert!(kept.kept_despite[0].contains(CAPTURE_B));
    assert_eq!(commit_count(&world.kb), commits + 1);
    assert_eq!(
        world.head_subject(),
        "入库：1 条（Claude，检查未通过，手动保留）"
    );
    assert_eq!(
        world.head_files(),
        [
            format!("A {RAW_NEW}"),
            format!("A {REPORT}"),
            format!("D {CAPTURE_A}"),
        ]
    );
    // The capture that never reached raw/ is back for the next run.
    assert_eq!(world.read(CAPTURE_B), capture_text("清单越短越好。"));
    assert_eq!(world.status_lines(), "?? 草稿.md");
    assert_eq!(world.lock.held_by(), None);
    assert_eq!(world.ended_notices(), [true]);
    let folder = world.dir.path().join("data/runs").join(&id);
    assert_eq!(record::load(&folder).unwrap().kept_despite, kept.kept_despite);
}

#[test]
fn an_engine_that_fails_midway_leaves_its_changes_for_a_decision_and_discard_undoes_them() {
    let world = World::new("run-midway");
    world.claude(
        r#"mkdir -p raw/inbox wiki/新目录
printf '# 收件箱\n\n## I-01\n周五做一次周回顾。\n\n## I-02\n清单越短越好。\n' > 'raw/inbox/新-收件箱.md'
printf '# 新页\n' > 'wiki/新目录/新页.md'
printf '# 小实验\n\n改到一半。\n' > 'wiki/概念/小实验.md'
echo 'something broke' >&2
exit 3
"#,
    );
    let commits = commit_count(&world.kb);

    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);
    let engine = record.engine_report.as_ref().unwrap();
    assert!(!engine.ok);
    assert_eq!(engine.exit.code, Some(3));
    assert_eq!(engine.exit.stderr_tail, "something broke");
    assert!(engine.failure.as_deref().unwrap().contains("退出码 3"));
    // Both captures did reach raw/, but an engine that failed is never committed.
    assert_eq!(World::rules(&record), ["engine"]);
    assert_eq!(commit_count(&world.kb), commits);
    assert!(world.auth_files().is_empty(), "the login file is gone after a failure too");
    let folder = world.dir.path().join("data/runs").join(&id);
    assert_eq!(
        fs::read_to_string(folder.join(record::STDERR_FILE)).unwrap(),
        "something broke"
    );

    world.service.discard(&id).unwrap();
    let discarded = world.finished(&id);
    assert_eq!(discarded.outcome, Some(Outcome::Discarded));
    assert_eq!(commit_count(&world.kb), commits);
    // Tracked files are as committed, what the run created is gone, the
    // user's untracked file was never touched.
    assert_eq!(world.status_lines(), "?? 草稿.md");
    assert_eq!(world.read("wiki/概念/小实验.md"), "# 小实验\n");
    assert!(!world.exists(RAW_NEW) && !world.exists("wiki/新目录"));
    assert_eq!(world.read("草稿.md"), "我自己的草稿\n");
    assert!(world.exists(CAPTURE_A) && world.exists(CAPTURE_B));
    // The run's files can still be taken from the copy.
    let backup = PathBuf::from(discarded.discarded_to.unwrap());
    assert_eq!(backup, folder.join(record::DISCARDED_DIR));
    assert!(fs::read_to_string(backup.join(RAW_NEW)).unwrap().contains("清单越短越好。"));
    assert_eq!(
        fs::read_to_string(backup.join("wiki/概念/小实验.md")).unwrap(),
        "# 小实验\n\n改到一半。\n"
    );
    assert_eq!(fs::read_to_string(backup.join("wiki/新目录/新页.md")).unwrap(), "# 新页\n");
    assert_eq!(world.lock.held_by(), None);
    assert_eq!(world.ended_notices(), [true]);
}

#[test]
fn a_missing_login_is_reported_clearly() {
    let world = World::new("run-login");
    world.claude(
        r#"printf '%s\n' '{"type":"assistant","error":"authentication_failed","message":{"content":[{"type":"text","text":"Not logged in · Please run /login"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login","total_cost_usd":0,"num_turns":1,"duration_ms":26,"permission_denials":[]}'
exit 1
"#,
    );
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Failed));
    let message = record.message.unwrap();
    assert!(message.starts_with("没有可用的登录凭据"), "{message}");
    assert_eq!(record.engine_report.unwrap().failure.as_deref(), Some(NOT_LOGGED_IN));
    assert!(world.exists(CAPTURE_A), "the queue is as it was");
    assert_eq!(world.status_lines(), "?? 草稿.md");
    assert_eq!(world.lock.held_by(), None);
    assert_eq!(world.ended_notices(), [false]);
    assert!(world.auth_files().is_empty());
}

#[test]
fn a_program_that_cannot_be_started_is_reported() {
    let world = World::new("run-no-program");
    relock(world.settings.lock()).claude_path = Some("/nonexistent/claude".into());
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Failed));
    let message = record.message.unwrap();
    assert!(message.contains("无法运行 /nonexistent/claude"), "{message}");
    assert!(message.contains("claudePath"), "{message}");
    assert!(world.auth_files().is_empty());
}

#[test]
fn stopping_a_run_kills_the_engine() {
    let world = World::new("run-cancel");
    world.claude(
        r#"printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"我要想很久。"}]}}'
sleep 60
"#,
    );
    let id = world.start(Task::Ingest);
    world.step_shown("我要想很久。");
    assert_eq!(world.service.status().phase, Phase::Running);
    // While the engine works, syncs and edits are told why they have to wait.
    let busy = world
        .lock
        .acquire("同步", Duration::from_secs(5))
        .err()
        .expect("refused during a run");
    assert_eq!(busy.operation, "入库");
    assert!(busy.to_string().contains("正在入库"), "{busy}");

    let began = Instant::now();
    world.service.cancel().unwrap();
    let record = world.finished(&id);
    assert!(began.elapsed() < Duration::from_secs(20));
    // It had not changed anything: there is nothing to decide.
    assert_eq!(record.outcome, Some(Outcome::Cancelled));
    let engine = record.engine_report.unwrap();
    assert!(engine.exit.cancelled);
    assert_eq!(engine.failure.as_deref(), Some("已手动停止"));
    assert!(world.exists(CAPTURE_A));
    assert_eq!(world.lock.held_by(), None);
    assert!(world.auth_files().is_empty());
    assert!(world.service.cancel().is_err(), "nothing left to stop");
}

#[test]
fn a_stopped_run_that_already_changed_files_waits_for_a_decision() {
    let world = World::new("run-cancel-changes");
    world.claude(
        r#"mkdir -p raw/inbox
printf '# 收件箱\n\n写到一半\n' > 'raw/inbox/新-收件箱.md'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"写了一半。"}]}}'
sleep 60
"#,
    );
    let id = world.start(Task::Ingest);
    world.step_shown("写了一半。");
    world.service.cancel().unwrap();
    let record = world.undecided(&id);
    assert!(World::rules(&record).contains(&"engine".to_string()));
    assert!(record
        .gate
        .unwrap()
        .violations
        .iter()
        .any(|violation| violation.message.contains("已手动停止")));
    assert!(world.exists(RAW_NEW));
    world.service.discard(&id).unwrap();
    assert!(!world.exists(RAW_NEW));
}

#[test]
fn a_run_that_takes_too_long_is_stopped() {
    let world = World::new("run-timeout");
    world.claude("sleep 60\n");
    relock(world.settings.lock()).timeout = Duration::from_millis(500);
    let began = Instant::now();
    let record = world.finished(&world.start(Task::Ingest));
    assert!(began.elapsed() < Duration::from_secs(20));
    assert_eq!(record.outcome, Some(Outcome::Failed));
    let engine = record.engine_report.unwrap();
    assert!(engine.exit.timed_out);
    assert!(engine.failure.unwrap().contains("没有结束，已停止"));
    assert!(record.message.unwrap().contains("没有留下任何改动"));
    assert_eq!(world.lock.held_by(), None);
}

#[test]
fn a_capture_made_during_the_run_is_its_own_commit_and_not_part_of_the_run() {
    let world = World::new("run-capture");
    let signal = world.dir.path().join("go-on");
    world.claude(&format!(
        r#"printf '%s\n' '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"等一下。"}}]}}}}'
i=0
while [ ! -f '{signal}' ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done
{INGEST}{RESULT}"#,
        signal = signal.display(),
    ));
    let commits = commit_count(&world.kb);
    let id = world.start(Task::Ingest);
    world.step_shown("等一下。");

    // 记一笔 while the engine works: the same steps the capture window takes.
    let fresh = {
        let guard = world
            .lock
            .acquire_beside_job("写入", Duration::from_secs(5))
            .expect("captures keep working");
        let built = capture::build_capture(
            "运行中记下的一条。",
            capture::MAC,
            &capture::LocalTime::now(),
        )
        .unwrap();
        let saved = capture::save_capture(&world.kb, &world.env.get().env, &built).unwrap();
        guard.note_commit(saved.outcome.commit.as_deref().unwrap(), &saved.path);
        saved.path
    };
    assert_eq!(commit_count(&world.kb), commits + 1);
    // An edit of an existing file is refused meanwhile.
    assert!(world.lock.acquire("写入", Duration::from_secs(5)).is_err());
    fs::write(&signal, "").unwrap();

    let record = world.finished(&id);
    assert_eq!(record.outcome, Some(Outcome::Committed), "{record:#?}");
    // Two commits: the capture, then the run. The run's commit knows nothing
    // of the capture, and the capture waits for the next run.
    assert_eq!(commit_count(&world.kb), commits + 2);
    assert_eq!(world.head_subject(), "入库：2 条（Claude）");
    assert!(!world.head_files().iter().any(|line| line.contains(&fresh)));
    assert_eq!(
        git(&world.kb, &["log", "-1", "--format=%s", "HEAD^"]),
        "收件箱：记一笔（mac）"
    );
    assert!(world.read(&fresh).ends_with("\n\n运行中记下的一条。\n"));
    assert_eq!(record.queue, [CAPTURE_A, CAPTURE_B]);
    assert!(record.gate.unwrap().violations.is_empty());
    assert_eq!(world.status_lines(), "?? 草稿.md");
}

#[test]
fn a_commit_made_by_the_engine_itself_is_a_violation() {
    let world = World::new("run-foreign");
    world.claude(&format!(
        "{INGEST}git add -A raw reports index.md && git commit -q -m '引擎自己提交的'\n{RESULT}"
    ));
    let commits = commit_count(&world.kb);
    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);
    assert_eq!(World::rules(&record), ["engineCommits"]);
    assert_eq!(commit_count(&world.kb), commits + 1, "only the engine's own commit");
    assert_eq!(world.head_subject(), "引擎自己提交的");

    // Discarding takes its changes back with one more commit, without rewriting history.
    world.service.discard(&id).unwrap();
    let discarded = world.finished(&id);
    assert_eq!(commit_count(&world.kb), commits + 2);
    assert_eq!(world.head_subject(), "撤回：AI 自己做的提交（入库）");
    assert_eq!(discarded.commit, Some(git(&world.kb, &["rev-parse", "HEAD"])));
    assert!(!world.exists(RAW_NEW));
    assert_eq!(world.read("index.md"), "# 总目录\n");
    assert_eq!(world.status_lines(), "?? 草稿.md");
}

#[test]
fn a_change_to_a_file_git_never_knew_is_a_violation_and_is_left_alone() {
    let world = World::new("run-untracked");
    world.claude(&format!("{INGEST}printf '被 AI 改了\\n' > '草稿.md'\n{RESULT}"));
    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);
    assert_eq!(World::rules(&record), ["untracked"]);
    world.service.discard(&id).unwrap();
    // The app neither commits nor restores what was never in git.
    assert_eq!(world.read("草稿.md"), "被 AI 改了\n");
    assert_eq!(world.status_lines(), "?? 草稿.md");
}

#[test]
fn without_an_answer_from_the_gate_nothing_is_committed_and_a_recheck_can_finish_the_run() {
    let world = World::with_timing(
        "run-silent-gate",
        RunTiming {
            gate_wait: Duration::from_millis(300),
            stop_grace: Duration::from_millis(300),
            lock_wait: Duration::from_secs(10),
        },
    );
    world.claude(&format!("{INGEST}{RESULT}"));
    world.set_gate(|_| None);
    let commits = commit_count(&world.kb);

    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);
    assert_eq!(record.gate, None);
    assert!(record.message.as_deref().unwrap().contains("没能完成检查"));
    assert_eq!(commit_count(&world.kb), commits);
    // Without a report there is nothing to keep.
    let refused = world.service.keep(&id).unwrap_err();
    assert!(refused.contains("先重新检查"), "{refused}");
    assert_eq!(world.service.status().phase, Phase::NeedsDecision);
    // A report that comes too late is not taken either.
    assert!(world
        .service
        .gate_report(&id, 1, honest_gate(&GateInput {
            run_id: id.clone(),
            attempt: 1,
            task: Task::Ingest,
            date: String::new(),
            changes: Vec::new(),
            unchanged: Vec::new(),
            queue: Vec::new(),
            final_message: None,
            engine_failure: None,
            foreign_commits: Vec::new(),
            untracked_changed: Vec::new(),
        }))
        .is_err());

    // The window is back: check again.
    world.set_gate(|input| Some(honest_gate(input)));
    world.service.recheck(&id).unwrap();
    let record = world.finished(&id);
    assert_eq!(record.outcome, Some(Outcome::Committed));
    assert_eq!(commit_count(&world.kb), commits + 1);
}

#[test]
fn a_report_that_does_not_fit_the_changes_is_not_used() {
    let world = World::new("run-bad-report");
    world.claude(&format!("{INGEST}{RESULT}"));
    world.set_gate(|input| {
        let mut report = honest_gate(input);
        // Says "ok" but leaves a change out, and wants a file that never changed.
        report.commit_paths = vec!["self/底色.md".to_string()];
        Some(report)
    });
    let commits = commit_count(&world.kb);
    let id = world.start(Task::Ingest);
    let record = world.undecided(&id);
    assert_eq!(record.gate, None);
    assert!(record.message.unwrap().contains("检查结果不能用"));
    assert_eq!(commit_count(&world.kb), commits);
    assert!(world.exists(CAPTURE_A));
}

#[test]
fn files_that_change_after_the_check_are_checked_again_before_the_commit() {
    let world = World::new("run-moved");
    world.claude(&format!("{INGEST}{RESULT}"));
    // The first answer comes while someone is still editing a file of the run.
    let calls = Arc::new(AtomicUsize::new(0));
    let kb = world.kb.clone();
    world.set_gate({
        let calls = Arc::clone(&calls);
        move |input| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                let path = kb.join(REPORT);
                let mut text = fs::read_to_string(&path).unwrap();
                text.push_str("\n检查期间补的一行。\n");
                fs::write(&path, text).unwrap();
            }
            Some(honest_gate(input))
        }
    });
    let record = world.finished(&world.start(Task::Ingest));
    assert_eq!(record.outcome, Some(Outcome::Committed));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "checked a second time");
    // What was committed is what was checked last.
    assert!(git(&world.kb, &["show", &format!("HEAD:{REPORT}")]).contains("检查期间补的一行。"));
    assert_eq!(world.status_lines(), "?? 草稿.md");
}

#[test]
fn keeping_is_refused_when_the_files_changed_since_the_check() {
    let world = World::new("run-keep-moved");
    world.claude(&format!("{HALF_INGEST}{RESULT}"));
    let commits = commit_count(&world.kb);
    let id = world.start(Task::Ingest);
    world.undecided(&id);
    // The user fixes the problem by hand: the missing capture goes into raw/.
    let mut raw = world.read(RAW_NEW);
    raw.push_str("\n## I-02\n清单越短越好。\n");
    fs::write(world.kb.join(RAW_NEW), raw).unwrap();

    let refused = world.service.keep(&id).unwrap_err();
    assert!(refused.contains("已经重新检查"), "{refused}");
    // The new check passes, so the run is committed like any good run.
    let record = world.finished(&id);
    assert_eq!(record.outcome, Some(Outcome::Committed), "{record:#?}");
    assert_eq!(commit_count(&world.kb), commits + 1);
    assert_eq!(world.head_subject(), "入库：2 条（Claude）");
    assert!(!world.exists(CAPTURE_B));
}

#[test]
fn only_one_run_at_a_time() {
    let world = World::new("run-one");
    world.claude(
        r#"printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"忙着。"}]}}'
sleep 60
"#,
    );
    let id = world.start(Task::Ingest);
    world.step_shown("忙着。");
    let refused = world
        .service
        .start(StartRequest {
            task: Task::LandRulings,
            overrides: Overrides::default(),
            trigger: "ui",
        })
        .unwrap_err();
    assert_eq!(refused, "正在入库，等它结束再开始新的");
    assert_eq!(world.engine_calls(), 1);
    world.service.cancel().unwrap();
    world.finished(&id);
    // Decisions about a run that is over are refused.
    assert!(world.service.discard(&id).is_err());
    assert!(world.service.keep(&id).is_err());
    assert!(world.service.recheck("no-such-run").is_err());
}

#[test]
fn land_rulings_works_through_the_answers() {
    let world = World::new("run-rulings");
    world.claude(&format!(
        r#"mkdir -p 'raw/对话'
printf '# 裁决\n\n## Q-004\n先不统一。\n' > 'raw/对话/新-裁决.md'
{RESULT}"#
    ));
    let id = world.start(Task::LandRulings);
    let record = world.finished(&id);
    assert_eq!(record.outcome, Some(Outcome::Committed), "{record:#?}");
    assert_eq!(record.queue, [ANSWER]);
    assert_eq!(world.head_subject(), "落裁决：1 份（Claude）");
    assert_eq!(
        world.head_files(),
        ["A raw/对话/新-裁决.md".to_string(), format!("D {ANSWER}")]
    );
    // The captures are another queue: untouched.
    assert!(world.exists(CAPTURE_A) && world.exists(CAPTURE_B));
    let prompt = world.recorded("args").unwrap();
    assert!(prompt.contains("本次任务：落裁决。"));
    assert!(prompt.contains(&format!("- {ANSWER}\n")));
    assert!(id.ends_with("-land-rulings"), "{id}");
}

#[test]
fn codex_gets_the_prompt_on_stdin_and_runs_without_the_users_mcp_servers() {
    let world = World::new("run-codex");
    // Codex can run commands: it removes the queue files itself.
    world.codex(&format!(
        r#"{INGEST}rm '{CAPTURE_A}' '{CAPTURE_B}'
printf '%s\n' '{{"type":"item.completed","item":{{"id":"i1","type":"file_change","changes":[{{"path":"'"$PWD"'/{RAW_NEW}","kind":"add"}}],"status":"completed"}}}}'
"#
    ));
    let id = world.start(Task::Ingest);
    let record = world.finished(&id);

    assert_eq!(record.outcome, Some(Outcome::Committed), "{record:#?}");
    assert_eq!(record.engine, EngineKind::Codex);
    assert_eq!(world.head_subject(), "入库：2 条（Codex）");
    assert_eq!(
        world.head_files(),
        [
            format!("A {RAW_NEW}"),
            format!("A {REPORT}"),
            format!("D {CAPTURE_A}"),
            format!("D {CAPTURE_B}"),
            "M index.md".to_string(),
        ]
    );
    let folder = world.dir.path().join("data/runs").join(&id);
    let args = world.recorded("args").unwrap();
    let expected = format!(
        "exec\n--json\n--sandbox\nworkspace-write\n-C\n{}\n-o\n{}\n-c\napproval_policy=\"never\"\n--disable\nplugins\n--disable\napps\n--disable\nhooks\n--disable\nmemories\n-c\nmcp_servers.chrome-devtools.enabled=false\n-c\nmcp_servers.notion.enabled=false\n-\n",
        world.kb.display(),
        folder.join(record::FINAL_MESSAGE_FILE).display(),
    );
    assert_eq!(args, expected);
    let prompt = world.recorded("stdin").unwrap();
    assert!(prompt.starts_with("你在我的个人决策底色知识库里工作。本次任务：入库。"));
    assert!(prompt.contains(&format!("- {CAPTURE_A}\n- {CAPTURE_B}\n")));
    // Its last reply comes from the file it was told to write.
    let engine = record.engine_report.unwrap();
    assert_eq!(
        engine.stream.final_message.as_deref(),
        Some(r#"{"任务": "入库", "条数": 2}"#)
    );
    assert_eq!(engine.stream.cost_usd, None);
    assert!(world.auth_files().is_empty(), "no login file for Codex");
    assert!(world
        .service
        .steps()
        .iter()
        .any(|event| event.step.verb.as_deref() == Some("写")
            && event.step.target.as_deref() == Some(RAW_NEW)));
}

#[test]
fn a_run_that_waited_for_a_decision_is_picked_up_again_after_a_restart() {
    let world = World::new("run-restore");
    world.claude(&format!("{HALF_INGEST}{RESULT}"));
    let id = world.start(Task::Ingest);
    world.undecided(&id);
    let commits = commit_count(&world.kb);

    // The app is closed and opened again: a new lock, a new service, the same folders.
    let lock = KbLock::default();
    let restarted = RunService::new(
        lock.clone(),
        world.env.clone(),
        RunDirs {
            runs: world.dir.path().join("data/runs"),
            temp: world.dir.path().join("tmp/dise"),
        },
        RunTiming {
            gate_wait: Duration::from_millis(200),
            stop_grace: Duration::from_millis(300),
            lock_wait: Duration::from_secs(10),
        },
        Hooks {
            root: Box::new({
                let kb = world.kb.clone();
                move || Ok(kb.clone())
            }),
            settings: Box::new(|| (RunSettings::default(), Vec::new())),
            sync: Box::new(|| SyncReport::stopped(SyncState::Synced, None)),
            // No window yet: nobody answers the check.
            notify: Box::new(|_| {}),
        },
    );
    // A login file an earlier crash left behind is cleaned up on the way.
    world.dir.write("tmp/dise/claude-auth-stale.json", "{}");
    restarted.restore();
    assert!(world.auth_files().is_empty());

    // From the first moment on the changes are protected again.
    assert!(lock.try_acquire("同步").is_err());
    let deadline = Instant::now() + Duration::from_secs(20);
    while restarted.status().phase != Phase::NeedsDecision {
        assert!(Instant::now() < deadline, "{:#?}", restarted.status());
        thread::sleep(Duration::from_millis(10));
    }
    let status = restarted.status();
    assert_eq!(status.run.as_ref().unwrap().id, id);
    assert_eq!(commit_count(&world.kb), commits);

    // And the decision still works: everything goes back.
    restarted.discard(&id).unwrap();
    assert_eq!(restarted.status().phase, Phase::Idle);
    assert_eq!(world.status_lines(), "?? 草稿.md");
    assert!(world.exists(CAPTURE_A) && world.exists(CAPTURE_B));
    assert!(lock.try_acquire("同步").is_ok());
    // Nothing is picked up a second time.
    restarted.restore();
    assert_eq!(restarted.status().phase, Phase::Idle);
}

/// A second service on the same app folders, as after a restart, working on `kb`.
fn restarted(world: &World, kb: PathBuf, lock: &KbLock) -> RunService {
    RunService::new(
        lock.clone(),
        world.env.clone(),
        RunDirs {
            runs: world.dir.path().join("data/runs"),
            temp: world.dir.path().join("tmp/dise"),
        },
        RunTiming {
            gate_wait: Duration::from_millis(200),
            stop_grace: Duration::from_millis(300),
            lock_wait: Duration::from_secs(10),
        },
        Hooks {
            root: Box::new(move || Ok(kb.clone())),
            settings: Box::new(|| (RunSettings::default(), Vec::new())),
            sync: Box::new(|| SyncReport::stopped(SyncState::Synced, None)),
            notify: Box::new(|_| {}),
        },
    )
}

#[test]
fn a_run_of_another_knowledge_base_folder_is_left_alone() {
    let world = World::new("run-restore-other");
    world.claude(&format!("{HALF_INGEST}{RESULT}"));
    let id = world.start(Task::Ingest);
    world.undecided(&id);

    // The app now works on another folder (kbPath changed, or another test
    // folder): the waiting run must not take that one over.
    let other = TempDir::new("run-restore-other-kb");
    init_repo(other.path());
    let lock = KbLock::default();
    let service = restarted(&world, other.path().to_path_buf(), &lock);
    service.restore();
    assert_eq!(service.status().phase, Phase::Idle);
    assert!(lock.try_acquire("同步").is_ok(), "nothing blocks the other folder");
    // The run is still waiting for its own folder.
    let (_, record) = record::unfinished(&world.dir.path().join("data/runs")).unwrap();
    assert_eq!(record.id, id);
}

#[test]
fn a_run_whose_start_commit_is_gone_is_given_up_instead_of_blocking() {
    let world = World::new("run-restore-gone");
    world.claude(&format!("{HALF_INGEST}{RESULT}"));
    let id = world.start(Task::Ingest);
    world.undecided(&id);
    // Somebody rewrote the history by hand meanwhile.
    let folder = world.dir.path().join("data/runs").join(&id);
    let mut saved = record::load_snapshot(&folder).unwrap();
    saved.snapshot.base = "0123456789abcdef0123456789abcdef01234567".into();
    record::save_snapshot(&folder, &saved).unwrap();

    let lock = KbLock::default();
    let service = restarted(&world, world.kb.clone(), &lock);
    service.restore();
    let deadline = Instant::now() + Duration::from_secs(20);
    while service.status().phase != Phase::Idle {
        assert!(Instant::now() < deadline, "{:#?}", service.status());
        thread::sleep(Duration::from_millis(10));
    }
    let record = service.status().run.unwrap();
    assert_eq!(record.outcome, Some(Outcome::Failed));
    assert!(record.message.unwrap().contains("运行开始时的提交已经不在"));
    assert!(lock.try_acquire("同步").is_ok());
    assert!(record::unfinished(&world.dir.path().join("data/runs")).is_none());
}

#[test]
fn quitting_stops_the_engine_and_the_next_start_finds_the_run() {
    let world = World::new("run-quit");
    world.claude(
        r#"mkdir -p raw/inbox
printf '# 收件箱\n\n写到一半\n' > 'raw/inbox/新-收件箱.md'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"还在写。"}]}}'
sleep 60
"#,
    );
    let id = world.start(Task::Ingest);
    world.step_shown("还在写。");
    let began = Instant::now();
    world.service.shutdown();
    let record = world.undecided(&id);
    assert!(began.elapsed() < Duration::from_secs(20));
    assert_eq!(
        record.engine_report.unwrap().failure.as_deref(),
        Some("App 退出时 AI 还没做完，被停止了")
    );
    assert!(world.auth_files().is_empty());
}

#[test]
fn statuses_and_steps_serialize_for_the_frontend() {
    let world = World::new("run-json");
    world.claude(&format!("{INGEST}{RESULT}"));
    let idle = serde_json::to_value(world.service.status()).unwrap();
    assert_eq!(
        idle,
        serde_json::json!({ "phase": "idle", "run": null, "gateAttempt": 0 })
    );
    let id = world.start(Task::Ingest);
    world.finished(&id);
    let status = serde_json::to_value(world.service.status()).unwrap();
    assert_eq!(status["phase"], "idle");
    assert_eq!(status["run"]["id"], id.as_str());
    assert_eq!(status["run"]["task"], "ingest");
    assert_eq!(status["run"]["outcome"], "committed");
    assert_eq!(status["run"]["engineReport"]["stream"]["costUsd"], 0.25);
    assert_eq!(status["run"]["gate"]["ok"], true);
    assert_eq!(status["run"]["gate"]["deleteQueuePaths"][0], CAPTURE_A);
    let step = serde_json::to_value(&world.service.steps()[0]).unwrap();
    assert_eq!(step["runId"], id.as_str());
    assert_eq!(step["seq"], 1);
    assert_eq!(step["kind"], "note");
    assert!(step["elapsedMs"].is_u64());
    assert!(step["text"].is_string());
    // The statuses a window saw on the way.
    let phases: Vec<Phase> = relock(world.notices.lock())
        .iter()
        .filter_map(|notice| match notice {
            Notice::Status(status) => Some(status.phase),
            _ => None,
        })
        .collect();
    for phase in [
        Phase::Starting,
        Phase::Running,
        Phase::Checking,
        Phase::Committing,
        Phase::Idle,
    ] {
        assert!(phases.contains(&phase), "{phase:?} missing in {phases:?}");
    }
}

#[test]
fn commit_messages_name_the_task_the_count_and_the_engine() {
    assert_eq!(
        commit_message(Task::Ingest, EngineKind::Claude, 3, false),
        "入库：3 条（Claude）"
    );
    assert_eq!(
        commit_message(Task::LandRulings, EngineKind::Codex, 2, false),
        "落裁决：2 份（Codex）"
    );
    assert_eq!(
        commit_message(Task::Ingest, EngineKind::Codex, 1, true),
        "入库：1 条（Codex，检查未通过，手动保留）"
    );
}

#[test]
fn the_engines_settings_are_looked_up_like_the_engines_do() {
    let home = ChildEnv::from_vars([("HOME", "/Users/someone")]);
    assert_eq!(
        engine_homes(&home),
        (
            PathBuf::from("/Users/someone/.claude/settings.json"),
            PathBuf::from("/Users/someone/.codex/config.toml")
        )
    );
    let moved = ChildEnv::from_vars([
        ("HOME", "/Users/someone"),
        ("CLAUDE_CONFIG_DIR", "/etc/claude"),
        ("CODEX_HOME", "/opt/codex-home"),
    ]);
    assert_eq!(
        engine_homes(&moved),
        (
            PathBuf::from("/etc/claude/settings.json"),
            PathBuf::from("/opt/codex-home/config.toml")
        )
    );
}

#[test]
fn engine_failures_are_put_in_plain_words() {
    let settings = RunSettings {
        budget_usd: 1.5,
        timeout: Duration::from_secs(1800),
        ..RunSettings::default()
    };
    let fail = |exit: Exit, stream: StreamSummary| {
        engine_failure(EngineKind::Claude, "claude", &exit, &stream, &settings, false)
    };
    let ended = |code: i32| Exit {
        code: Some(code),
        ..Exit::default()
    };
    let finished = || StreamSummary {
        finished: true,
        ..StreamSummary::default()
    };
    assert_eq!(fail(ended(0), finished()), None);
    assert_eq!(
        fail(
            Exit {
                timed_out: true,
                ..Exit::default()
            },
            StreamSummary::default()
        )
        .as_deref(),
        Some("超过 30 分钟没有结束，已停止")
    );
    assert!(fail(
        ended(0),
        StreamSummary {
            is_error: true,
            subtype: Some("error_max_budget_usd".into()),
            ..finished()
        }
    )
    .unwrap()
    .contains("上限（1.5 美元）"));
    assert_eq!(
        fail(
            ended(1),
            StreamSummary {
                is_error: true,
                error: Some("API Error: 529 overloaded".into()),
                ..finished()
            }
        )
        .as_deref(),
        Some("AI 报告了错误：API Error: 529 overloaded")
    );
    assert_eq!(
        fail(ended(1), StreamSummary::default()).as_deref(),
        Some("AI 没有给出结果就退出了（退出码 1）")
    );
    assert_eq!(
        fail(ended(2), finished()).as_deref(),
        Some("AI 没有正常退出（退出码 2）")
    );
    assert!(fail(
        Exit {
            spawn_error: Some("No such file or directory".into()),
            ..Exit::default()
        },
        StreamSummary::default()
    )
    .unwrap()
    .contains("claudePath"));
}
