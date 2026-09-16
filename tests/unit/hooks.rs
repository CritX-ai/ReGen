//! Re-enter this native test executable as a hook: no shell or external tools.

use super::{run_post, run_pre};
use crate::config::{HookWhen, Hooks, Minify, PostHook, PreHook, ResolvedBuild};
use crate::test_support::tempdir;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn settings(pre: Vec<PreHook>, post: Vec<PostHook>) -> ResolvedBuild {
    ResolvedBuild {
        profile: "preview".to_owned(),
        review: true,
        minify: Minify {
            html: false,
            css: false,
            js: false,
            html_options: Default::default(),
            css_options: Default::default(),
            js_options: Default::default(),
        },
        minify_assets: false,
        regression_checks: crate::RegressionCheckMode::Off,
        hooks: Hooks { pre, post },
    }
}

fn command(action: &str) -> Vec<String> {
    vec![
        std::env::current_exe()
            .unwrap()
            .into_os_string()
            .into_string()
            .expect("test executable path is UTF-8"),
        "--exact".to_owned(),
        format!(
            "{}::hook_process",
            module_path!().split_once("::").unwrap().1
        ),
        "--ignored".to_owned(),
        "--nocapture".to_owned(),
        // Libtest accepts a literal filter argument, which the fixture can read
        // without introducing a shell, another executable, or global env writes.
        "--skip".to_owned(),
        action.to_owned(),
    ]
}

fn pre(action: &str, allow_failure: bool) -> PreHook {
    PreHook {
        command: command(action),
        allow_failure,
    }
}

fn post(action: &str, when: HookWhen, error_details: bool, allow_failure: bool) -> PostHook {
    PostHook {
        command: command(action),
        when,
        error_details,
        allow_failure,
    }
}

fn events(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
#[ignore = "native child-process fixture, invoked only by hook runner tests"]
fn hook_process() {
    let argv: Vec<String> = std::env::args().collect();
    let action = argv.last().unwrap();
    // Coverage runs include ignored tests; only a runner-spawned child has this
    // environment. Parent tests verify the child's actual filesystem effects.
    let Some(site) = std::env::var_os("REGEN_SITE") else {
        return;
    };
    let root = Path::new(&site);
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    assert!(input.is_empty(), "hooks must not consume parent stdin");
    let event = serde_json::json!({
        "action": action,
        "cwd": std::env::current_dir().unwrap(),
        "site": root,
        "output": std::env::var_os("REGEN_OUTPUT").map(PathBuf::from),
        "installed": std::env::var_os("REGEN_OUTPUT")
            .is_some_and(|path| Path::new(&path).join("index.html").is_file()),
        "profile": std::env::var("REGEN_PROFILE").unwrap(),
        "status": std::env::var("REGEN_STATUS").unwrap(),
        "error": std::env::var("REGEN_ERROR").ok(),
        "truncated": std::env::var("REGEN_ERROR_TRUNCATED").ok(),
    });
    // Append real side effects before returning failure so callers must preserve
    // ordered execution and cannot confuse hook failure with absence of effects.
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("events.jsonl"))
        .unwrap();
    serde_json::to_writer(&mut output, &event).unwrap();
    writeln!(output).unwrap();
    drop(output);
    if action.starts_with("generate") {
        fs::create_dir_all(root.join("content/en/pages")).unwrap();
        fs::create_dir_all(root.join("templates")).unwrap();
        fs::write(root.join("content/en/site.yaml"), "{}").unwrap();
        let title = if action == "generate-update" {
            "updated"
        } else {
            "generated"
        };
        fs::write(
            root.join("content/en/pages/index.yaml"),
            format!("title: {title}\ndescription: test\ntemplate: page.html\nslug: \"\"\n"),
        )
        .unwrap();
        fs::write(
            root.join("templates/page.html"),
            "<main>{{ page.title }}</main>",
        )
        .unwrap();
    }
    if action.starts_with("fail-") {
        std::process::exit(23);
    }
}

#[test]
fn optional_pre_failure_continues_but_required_failure_stops_the_sequence() {
    let temp = tempdir();
    let root = temp.path().canonicalize().unwrap();
    let output = root.join("review");
    let literal = "literal $REGEN_SITE ; \"quotes\" &";
    let build = settings(
        vec![
            pre("fail-allowed", true),
            pre(literal, false),
            pre("fail-required", false),
            pre("must-not-run", false),
        ],
        vec![post("after-pre-failure", HookWhen::Failure, false, false)],
    );
    let error = run_pre(&build, &root, &output).unwrap_err();
    run_post(&build, &root, &output, Some(&error)).unwrap();
    let events = events(&root);
    let actions: Vec<_> = events
        .iter()
        .map(|event| event["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "fail-allowed",
            literal,
            "fail-required",
            "after-pre-failure"
        ]
    );
    for event in &events {
        assert_eq!(event["cwd"], serde_json::to_value(&root).unwrap());
        assert_eq!(event["site"], serde_json::to_value(&root).unwrap());
        assert_eq!(event["output"], serde_json::to_value(&output).unwrap());
        assert_eq!(event["profile"], "preview");
        assert!(event["error"].is_null());
        assert!(event["truncated"].is_null());
    }
    assert!(events[..3].iter().all(|event| event["status"] == "pre"));
    assert_eq!(events[3]["status"], "failure");
}

#[test]
fn all_eligible_post_hooks_run_without_reclassifying_a_successful_build() {
    let temp = tempdir();
    let root = temp.path().canonicalize().unwrap();
    let build = settings(
        vec![],
        vec![
            post("fail-first", HookWhen::Success, false, false),
            post("wrong-outcome", HookWhen::Failure, false, false),
            post("fail-second", HookWhen::Always, false, false),
            post("after-failures", HookWhen::Success, false, false),
        ],
    );
    assert!(run_post(&build, &root, &root.join("review"), None).is_err());
    let events = events(&root);
    let actions: Vec<_> = events
        .iter()
        .map(|event| event["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["fail-first", "fail-second", "after-failures"]);
    assert!(events.iter().all(|event| event["status"] == "success"));
}

#[test]
fn optional_post_failures_do_not_fail_a_successful_build() {
    let temp = tempdir();
    let root = temp.path().canonicalize().unwrap();
    let build = settings(
        vec![],
        vec![
            post("fail-optional", HookWhen::Always, false, true),
            post("after-optional", HookWhen::Success, false, false),
        ],
    );
    run_post(&build, &root, &root.join("review"), None).unwrap();
    let events = events(&root);
    assert_eq!(events[0]["action"], "fail-optional");
    assert_eq!(events[1]["action"], "after-optional");
    assert_eq!(events.len(), 2);
}

#[test]
fn spawn_errors_keep_the_original_io_cause_after_context_and_post_aggregation() {
    let temp = tempdir();
    let root = temp.path().canonicalize().unwrap();
    let missing = root.join("missing-program").to_str().unwrap().to_owned();
    let build = settings(
        vec![PreHook {
            command: vec![missing.clone()],
            allow_failure: false,
        }],
        vec![
            PostHook {
                command: vec![missing],
                when: HookWhen::Always,
                error_details: false,
                allow_failure: false,
            },
            post("fail-after-spawn", HookWhen::Always, false, false),
        ],
    );
    let error = run_pre(&build, &root, &root.join("review")).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    let error = run_post(&build, &root, &root.join("review"), None).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(
        format!("{error:#}").contains("23"),
        "later exit status must remain in diagnostics"
    );
    assert_eq!(events(&root)[0]["action"], "fail-after-spawn");
}

#[test]
fn inherited_error_variables_are_removed_and_failure_details_are_bounded_opt_in() {
    let temp = tempdir();
    let root = temp.path().canonicalize().unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!(
                "{}::inherited_error_process",
                module_path!().split_once("::").unwrap().1
            ),
            "--ignored",
            "--nocapture",
        ])
        .env("REGEN_HOOK_TEST_ROOT", &root)
        .env("REGEN_ERROR", "inherited secret")
        .env("REGEN_ERROR_TRUNCATED", "inherited marker")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let events = events(&root);
    assert_eq!(events.len(), 6);
    for index in [0, 1, 3] {
        assert!(events[index]["error"].is_null());
        assert!(events[index]["truncated"].is_null());
    }
    assert_eq!(events[0]["status"], "pre");
    assert_eq!(events[1]["status"], "success");
    assert_eq!(events[2]["status"], "failure");
    assert_eq!(events[2]["error"], "build context: original cause");
    assert_eq!(events[2]["truncated"], "0");
    assert_eq!(events[3]["status"], "failure");
    let bounded = events[4]["error"].as_str().unwrap();
    let original = format!("build context: {}", "€".repeat(4000));
    assert!(original.starts_with(bounded));
    assert!((8190..=8192).contains(&bounded.len()));
    assert_eq!(events[4]["truncated"], "1");
    assert_eq!(events[5]["error"].as_str().unwrap(), "x".repeat(8192));
    assert_eq!(events[5]["truncated"], "0");
}

#[test]
#[ignore = "isolated environment parent, invoked only by the privacy test"]
fn inherited_error_process() {
    let Some(root) = std::env::var_os("REGEN_HOOK_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let output = root.join("review");
    let build = settings(
        vec![pre("private-pre", false)],
        vec![post("private-success", HookWhen::Always, true, false)],
    );
    run_pre(&build, &root, &output).unwrap();
    run_post(&build, &root, &output, None).unwrap();

    let build = settings(
        vec![],
        vec![
            post("fail-with-details", HookWhen::Failure, true, false),
            post("private-failure", HookWhen::Always, false, false),
            post("wrong-success", HookWhen::Success, true, false),
        ],
    );
    let error = anyhow::anyhow!("original cause").context("build context");
    assert!(run_post(&build, &root, &output, Some(&error)).is_err());
    let build = settings(
        vec![],
        vec![post("bounded-details", HookWhen::Failure, true, false)],
    );
    let error = anyhow::anyhow!("€".repeat(4000)).context("build context");
    run_post(&build, &root, &output, Some(&error)).unwrap();
    let exact = anyhow::anyhow!("x".repeat(8192));
    run_post(&build, &root, &output, Some(&exact)).unwrap();
}

fn configure(root: &Path, pre: &[PreHook], post: &[PostHook]) {
    use std::fmt::Write as _;
    let mut config = String::from(
        "[site]\ntitle = \"Hooks\"\nbase_url = \"https://example.com\"\ndefault_language = \"en\"\n\
         [[languages]]\ncode = \"en\"\nname = \"English\"\n",
    );
    for hook in pre {
        writeln!(
            config,
            "[[build.hooks.pre]]\ncommand = {}\nallow_failure = {}",
            serde_json::to_string(&hook.command).unwrap(),
            hook.allow_failure,
        )
        .unwrap();
    }
    for hook in post {
        writeln!(
            config,
            "[[build.hooks.post]]\ncommand = {}\nwhen = {}\nerror_details = {}\nallow_failure = {}",
            serde_json::to_string(&hook.command).unwrap(),
            serde_json::to_string(&hook.when).unwrap(),
            hook.error_details,
            hook.allow_failure,
        )
        .unwrap();
    }
    fs::write(root.join("regen.toml"), config).unwrap();
}

#[test]
fn configured_hooks_prepare_inputs_and_observe_installed_output() {
    let site = tempdir();
    let root = site.path();
    configure(
        root,
        &[pre("generate", false)],
        &[post("published", HookWhen::Success, false, false)],
    );
    let built = crate::build(root).unwrap();
    assert!(
        fs::read_to_string(built.output.join("index.html"))
            .unwrap()
            .contains("<main>generated</main>")
    );
    let first = events(root);
    assert_eq!(first[0]["action"], "generate");
    assert_eq!(first[0]["installed"], false);
    assert_eq!(first[1]["action"], "published");
    assert_eq!(first[1]["installed"], true);
    fs::remove_file(root.join("events.jsonl")).unwrap();
    let reviewed = crate::build_with_options(
        root,
        &crate::BuildOptions {
            review: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(reviewed.output, root.join("review"));
    let review_events = events(root);
    assert_eq!(review_events[0]["installed"], false);
    assert_eq!(review_events[1]["installed"], true);
    for event in &review_events {
        assert_eq!(event["profile"], "release");
        assert_eq!(event["output"], reviewed.output.to_str().unwrap());
    }
    assert_eq!(
        fs::read(built.output.join("index.html")).unwrap(),
        fs::read(reviewed.output.join("index.html")).unwrap()
    );
    fs::remove_file(root.join("events.jsonl")).unwrap();
    configure(
        root,
        &[pre("generate-update", false)],
        &[
            post("fail-required-post", HookWhen::Success, false, false),
            post("wrong-failure", HookWhen::Failure, true, false),
            post("final-success", HookWhen::Always, true, false),
        ],
    );
    assert!(crate::build(root).is_err());
    assert_eq!(
        fs::read_to_string(built.output.join("index.html")).unwrap(),
        "<main>updated</main>"
    );
    let updated = events(root);
    assert_eq!(
        updated
            .iter()
            .map(|event| event["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["generate-update", "fail-required-post", "final-success"]
    );
    assert_eq!(updated[2]["status"], "success");
    assert!(updated[2]["error"].is_null());
    assert!(!root.join(".regen-stage").exists());
    assert!(!root.join(".regen-previous").exists());
}

#[cfg(feature = "minify-html")]
#[test]
fn warning_only_regressions_install_output_and_notify_success_hooks() {
    let site = tempdir();
    let root = site.path();
    configure(root, &[pre("generate", false)], &[]);
    crate::build(root).unwrap();
    fs::remove_file(root.join("events.jsonl")).unwrap();
    configure(
        root,
        &[],
        &[
            post("success", HookWhen::Success, true, false),
            post("failure", HookWhen::Failure, true, false),
            post("always", HookWhen::Always, true, false),
        ],
    );
    fs::write(
        root.join("templates/page.html"),
        "<!doctype html><title>Warning</title><p><input type=\"text\"></p>",
    )
    .unwrap();
    let config_path = root.join("regen.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        config_path,
        format!(
            "{config}\n[build.minify]\nhtml = true\n\
             [build.minify.html_options]\nremove_input_type_text = true\n"
        ),
    )
    .unwrap();
    let built = crate::build_with_options(
        root,
        &crate::BuildOptions {
            regression_checks: Some(crate::RegressionCheckMode::Warn),
            ..Default::default()
        },
    )
    .unwrap();
    let html = fs::read_to_string(built.output.join("index.html")).unwrap();
    assert!(!html.contains("type="));
    assert!(html.contains("<input>"));
    let log = events(root);
    assert_eq!(
        log.iter()
            .map(|event| event["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["success", "always"]
    );
    for event in log {
        assert_eq!(event["status"], "success");
        assert_eq!(event["installed"], true);
        assert!(event["error"].is_null());
    }
}

#[test]
fn failure_hooks_preserve_primary_errors_and_transport_nul_details() {
    let site = tempdir();
    let root = site.path();
    configure(root, &[pre("generate", false)], &[]);
    let built = crate::build(root).unwrap();
    let previous = fs::read(built.output.join("regen-manifest.json")).unwrap();
    let missing = PostHook {
        command: vec![root.join("no-hook-program").to_str().unwrap().to_owned()],
        when: HookWhen::Failure,
        error_details: false,
        allow_failure: false,
    };
    configure(
        root,
        &[],
        &[
            missing,
            post("report-failure", HookWhen::Failure, true, false),
        ],
    );
    fs::create_dir(root.join(".regen-stage")).unwrap();
    let error = crate::build(root).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert!(root.join(".regen-stage").is_dir());
    assert_eq!(
        fs::read(built.output.join("regen-manifest.json")).unwrap(),
        previous
    );
    fs::remove_dir(root.join(".regen-stage")).unwrap();

    configure(
        root,
        &[],
        &[post("report-nul", HookWhen::Failure, true, false)],
    );
    fs::write(
        root.join("content/en/pages/index.yaml"),
        "title: Bad\ndescription: test\ntemplate: page.html\nslug: \"bad\\0route\"\n",
    )
    .unwrap();
    assert!(crate::build(root).is_err());
    let log = events(root);
    let last = log.last().unwrap();
    assert_eq!(last["action"], "report-nul");
    assert_eq!(last["status"], "failure");
    let details = last["error"].as_str().unwrap();
    assert!(details.contains("bad\\0route"));
    assert!(!details.contains('\0'));
    assert_eq!(
        fs::read(built.output.join("regen-manifest.json")).unwrap(),
        previous
    );

    configure(
        root,
        &[pre("fail-required-pre", false)],
        &[post("report-pre", HookWhen::Failure, true, false)],
    );
    let error = crate::build(root).unwrap_err();
    assert!(format!("{error:#}").contains("23"));
    let log = events(root);
    assert_eq!(log.last().unwrap()["action"], "report-pre");
    assert_eq!(
        fs::read(built.output.join("regen-manifest.json")).unwrap(),
        previous
    );
}

#[test]
fn invalid_build_settings_cannot_start_hooks_or_replace_output() {
    let site = tempdir();
    let root = site.path();
    configure(root, &[pre("generate", false)], &[]);
    let built = crate::build(root).unwrap();
    let previous = fs::read(built.output.join("regen-manifest.json")).unwrap();
    fs::remove_file(root.join("events.jsonl")).unwrap();
    configure(
        root,
        &[pre("generate-update", false)],
        &[post("must-not-run", HookWhen::Always, true, false)],
    );
    assert!(
        crate::build_with_options(
            root,
            &crate::BuildOptions {
                profile: Some("../outside"),
                ..Default::default()
            }
        )
        .is_err()
    );
    configure(
        root,
        &[PreHook {
            command: vec![],
            allow_failure: false,
        }],
        &[post("must-not-run", HookWhen::Always, true, false)],
    );
    assert!(crate::build(root).is_err());
    assert!(!root.join("events.jsonl").exists());
    assert!(!root.join(".regen-stage").exists());
    assert_eq!(
        fs::read(built.output.join("regen-manifest.json")).unwrap(),
        previous
    );
}

#[test]
fn nul_expansion_at_the_error_limit_still_delivers_a_bounded_failure_notification() {
    let site = tempdir();
    let root = site.path();
    let build = settings(
        vec![],
        vec![post("bounded-nul", HookWhen::Failure, true, false)],
    );
    // The escape itself crosses the bound, then the escape fits but its suffix
    // does not. Both paths must start the notifier with a valid environment.
    for prefix in [8191, 8190] {
        let error = anyhow::anyhow!(format!("{}\0€", "x".repeat(prefix)));
        run_post(&build, root, &root.join("review"), Some(&error)).unwrap();
    }
    let log = events(root);
    assert_eq!(log[0]["error"], format!("{}\\", "x".repeat(8191)));
    assert_eq!(log[1]["error"], format!("{}\\0", "x".repeat(8190)));
    assert!(
        log.iter()
            .all(|event| event["truncated"] == "1" && event["status"] == "failure")
    );
}
