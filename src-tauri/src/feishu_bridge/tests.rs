use super::*;

fn bot() -> Bot {
    Bot {
        id: "test".into(),
        name: "Test".into(),
        app_id: "cli_test".into(),
        tenant_key: "tenant".into(),
        allowed_users: vec!["ou_owner".into()],
        allowed_groups: vec!["oc_group".into()],
        allow_private: true,
        engine: Engine::Codex,
        projects: vec![],
    }
}

fn message() -> IncomingMessage {
    IncomingMessage {
        message_id: "om_message".into(),
        chat_id: "oc_private".into(),
        sender_open_id: "ou_owner".into(),
        tenant_key: "tenant".into(),
        text: "/projects".into(),
        is_group: false,
        mentions_bot: false,
    }
}

#[test]
fn requires_tenant_user_and_chat_authorization() {
    let config = bot();
    let mut event = message();
    assert!(authorized(&config, &event));
    event.tenant_key = "other".into();
    assert!(!authorized(&config, &event));
    event.tenant_key = "tenant".into();
    event.sender_open_id = "ou_other".into();
    assert!(!authorized(&config, &event));
    event.sender_open_id = "ou_owner".into();
    event.is_group = true;
    assert!(!authorized(&config, &event));
    event.chat_id = "oc_group".into();
    assert!(!authorized(&config, &event));
    event.mentions_bot = true;
    assert!(authorized(&config, &event));
    let mut state = ChatState::default();
    assert!(
        !respond(&config, &event, &mut state)
            .unwrap()
            .contains("Projects")
    );
}

#[test]
fn path_rules_reject_traversal_links_and_sensitive_files() {
    let root = tempfile::tempdir().unwrap();
    let root = canonical_root(&root.path().canonicalize().unwrap()).unwrap();
    for path in [
        "../secret",
        "/absolute",
        "C:drive",
        ".git/config",
        ".env",
        "x/.env",
        "x/../file",
        "CON.txt",
        "f. ",
        "safe//f",
        "credentials.json",
        "id_ed25519",
    ] {
        assert!(resolve(&root, path, false).is_err(), "{path}");
    }
    fs::write(root.join("original.txt"), "outside").unwrap();
    fs::hard_link(root.join("original.txt"), root.join("linked.txt")).unwrap();
    assert!(resolve(&root, "linked.txt", true).is_err());
}

#[test]
fn mutations_are_separate_and_deletion_is_one_time_and_revision_bound() {
    let root = tempfile::tempdir().unwrap();
    let mut config = bot();
    let root = canonical_root(&root.path().canonicalize().unwrap()).unwrap();
    config.projects.push(ProjectGrant {
        path: root.to_string_lossy().into_owned(),
        read: true,
        create: false,
        edit: false,
        delete: true,
    });
    fs::write(root.join("a.txt"), "original").unwrap();
    let mut event = message();
    let mut state = ChatState::default();
    event.text = "/project 1".into();
    respond(&config, &event, &mut state).unwrap();
    event.text = "/create b.txt\nnew".into();
    assert!(respond(&config, &event, &mut state).is_err());
    event.text = "/edit a.txt\ninvalid\nnew".into();
    assert!(respond(&config, &event, &mut state).is_err());
    event.text = "/delete a.txt".into();
    respond(&config, &event, &mut state).unwrap();
    let ticket = *state.pending.keys().next().unwrap();
    event.text = format!("/confirm {ticket}");
    event.chat_id = "oc_other".into();
    assert!(respond(&config, &event, &mut state).is_err());
    assert!(root.join("a.txt").exists());
    event.chat_id = "oc_private".into();
    fs::write(root.join("a.txt"), "changed").unwrap();
    assert!(respond(&config, &event, &mut state).is_err());
    fs::write(root.join("a.txt"), "original").unwrap();
    assert!(respond(&config, &event, &mut state).is_err());
    assert!(root.join("a.txt").exists());
}

#[test]
fn file_queries_stay_in_selected_chat_and_project() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(root.join("top.txt"), "first project").unwrap();
    fs::write(nested.join("inside.txt"), "second project").unwrap();
    fs::write(root.join("bad.txt"), "api_key=secretvalue").unwrap();
    let mut config = bot();
    for path in [&root, &nested] {
        config.projects.push(ProjectGrant {
            path: path.to_string_lossy().into_owned(),
            read: true,
            create: false,
            edit: false,
            delete: false,
        });
    }
    let mut state = ChatState {
        boundaries: vec![root.clone(), nested],
        ..ChatState::default()
    };
    let mut event = message();
    event.text = "/project 1".into();
    respond(&config, &event, &mut state).unwrap();
    event.text = "/read top.txt".into();
    assert!(
        respond(&config, &event, &mut state)
            .unwrap()
            .contains("first project")
    );
    event.sender_open_id = "ou_other".into();
    assert!(respond(&config, &event, &mut state).is_err());
    event.sender_open_id = "ou_owner".into();
    event.text = "/ask nested/inside.txt\nExplain it".into();
    assert!(prepare_ask(&config, &event, &state).is_err());
    event.text = "/ask bad.txt\nExplain it".into();
    assert!(prepare_ask(&config, &event, &state).is_err());
    event.text = "/ask top.txt\nExplain it".into();
    let request = prepare_ask(&config, &event, &state).unwrap();
    assert_eq!(request.source, "first project");
    assert_eq!(request.question, "Explain it");
    assert_eq!(request.relative, "top.txt");
}

#[test]
fn create_never_overwrites_and_edit_requires_current_revision() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut config = bot();
    config.projects.push(ProjectGrant {
        path: root.to_string_lossy().into_owned(),
        read: false,
        create: true,
        edit: true,
        delete: false,
    });
    let mut state = ChatState::default();
    let mut event = message();
    event.text = "/project 1".into();
    respond(&config, &event, &mut state).unwrap();
    event.text = "/create file.txt\noriginal  \n".into();
    respond(&config, &event, &mut state).unwrap();
    let target = root.join("file.txt");
    assert_eq!(fs::read_to_string(&target).unwrap(), "original  \n");
    event.text = "/create file.txt\nreplacement".into();
    assert!(respond(&config, &event, &mut state).is_err());
    event.text = "/read file.txt".into();
    assert!(respond(&config, &event, &mut state).is_err());
    event.text = "/edit file.txt\nstale\nreplacement".into();
    assert!(respond(&config, &event, &mut state).is_err());
    let (_, revision) = read_file(&target).unwrap();
    event.text = format!("/edit file.txt\n{revision}\nreplacement");
    respond(&config, &event, &mut state).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "replacement");
}

#[test]
fn audit_records_identity_scope_operation_and_outcome_without_message_body() {
    let directory = tempfile::tempdir().unwrap();
    let app = Arc::new(AppState::with_home(
        crate::paths::AgentPaths::from_home(directory.path()),
        directory.path().to_path_buf(),
        directory.path().to_path_buf(),
    ));
    let bridge = Bridge::new(app);
    let mut event = message();
    event.text = "/edit private/path.txt\nsecret contents".into();
    bridge.audit(&bot(), &event, Some("registered project"), "/edit", false);
    let text = fs::read_to_string(directory.path().join("feishu-audit.jsonl")).unwrap();
    let record: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(record["actor"], "ou_owner");
    assert_eq!(record["project"], "registered project");
    assert_eq!(record["operation"], "/edit");
    assert_eq!(record["outcome"], "error");
    assert!(!text.contains("secret contents") && !text.contains("private/path.txt"));
}

#[cfg(unix)]
#[test]
fn fifo_is_rejected_before_opening() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let fifo = root.join("wait");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // The nul-terminated path remains alive for the duration of this call.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(resolve(&root, "wait", true).is_err());
    assert!(open_file(&fifo).is_err());
}

#[test]
fn natural_project_matching_distinguishes_names_boundaries_and_duplicates() {
    use conversation::named_projects;
    let names = ["cc", "cc-memory", "库存工具", "cc-memory"];
    assert_eq!(
        named_projects("帮我看看 cc-memory 的登录问题", &names),
        vec![1, 3]
    );
    assert_eq!(named_projects("检查库存工具的登录", &names), vec![2]);
    assert!(named_projects("看看 gcc 的设置", &names).is_empty());
    assert_eq!(named_projects("检查 cc 和库存工具", &names), vec![0, 2]);
}

#[test]
fn project_agent_operations_use_the_same_permissions_and_path_validation() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = bot();
    let temp_root = dir.path().canonicalize().unwrap();
    let root = canonical_root(&temp_root).unwrap();
    config.projects.push(ProjectGrant {
        path: root.to_string_lossy().into_owned(),
        read: true,
        create: false,
        edit: false,
        delete: false,
    });
    fs::write(root.join("code.txt"), "project code").unwrap();
    fs::write(root.join(".env"), "project code").unwrap();
    let mut event = message();
    let mut state = ChatState::default();
    event.text = "/project 1".into();
    respond(&config, &event, &mut state).unwrap();
    let op = |action, path: &str| conversation::Step {
        action,
        path: Some(path.to_owned()),
        text: None,
        revision: None,
    };
    use conversation::Action::*;
    assert!(
        conversation::execute(&config, &event, &mut state, op(Read, "code.txt"))
            .unwrap()
            .contains("project code")
    );
    assert!(conversation::execute(&config, &event, &mut state, op(Read, "../outside")).is_err());
    assert!(conversation::execute(&config, &event, &mut state, op(Create, "new.txt")).is_err());
    assert!(conversation::execute(&config, &event, &mut state, op(Delete, "code.txt")).is_err());
    assert!(root.join("code.txt").is_file());
    let search = conversation::Step {
        action: Search,
        path: None,
        text: Some("project code".into()),
        revision: None,
    };
    assert_eq!(
        conversation::execute(&config, &event, &mut state, search).unwrap(),
        "code.txt:1"
    );
}

#[test]
fn model_routing_only_accepts_bound_projects_and_preserves_ambiguity() {
    let mut config = bot();
    for path in ["workspace/cc-memory", "workspace/inventory"] {
        config.projects.push(ProjectGrant {
            path: path.into(),
            read: true,
            create: false,
            edit: false,
            delete: false,
        });
    }
    let choose = |text, current, result: &str| {
        conversation::choose(&config, text, current, |_| Ok(result.to_owned()))
    };
    assert_eq!(choose("请检查库存服务", Some(0), r#"{"project":2}"#), Ok(1));
    assert_eq!(choose("那继续", Some(1), r#"{"project":2}"#), Ok(1));
    assert!(choose("检查另一个未绑定项目", Some(0), r#"{"project":null}"#).is_err());
    assert!(choose("看看代码", None, r#"{"project":3}"#).is_err());
    assert!(choose("比较 inventory 和 cc-memory", None, r#"{"project":1}"#).is_err());
}

#[test]
fn conversation_passes_tool_results_and_keeps_only_its_own_history() {
    use serde_json::json;
    let grant = ProjectGrant {
        path: "workspace/project-a".into(),
        read: true,
        create: false,
        edit: false,
        delete: false,
    };
    let mut first = conversation::History::new();
    first.push_back(json!({"user": "remember project A"}));
    let second = conversation::History::new();
    let cancel = AtomicBool::new(false);
    let mut calls = 0;
    let result = conversation::turn(
        &grant,
        &mut first,
        "read config",
        &cancel,
        |prompt| {
            assert!(prompt.contains("remember project A"));
            calls += 1;
            if calls == 1 {
                Ok(r#"{"action":"read","path":"config.ts"}"#.into())
            } else {
                assert!(prompt.contains("bounded evidence"));
                Ok(r#"{"action":"answer","text":"Found it"}"#.into())
            }
        },
        |step| {
            assert_eq!(step.path.as_deref(), Some("config.ts"));
            Ok("bounded evidence".into())
        },
    );
    assert_eq!(result, Ok("Found it".into()));
    assert_eq!(first.len(), 2);
    assert!(second.is_empty());
}
