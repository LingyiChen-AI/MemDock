use super::*;

#[test]
fn saves_text_without_overwriting_an_external_change() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("code.ts");
    fs::write(&path, b"\xef\xbb\xbflet x = 1;\r\n").unwrap();
    let source = read_text(&path).unwrap();
    assert_eq!(source.text, "let x = 1;\n");
    save_text(&path, &source.revision, "let x = 2;\n").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbflet x = 2;\r\n");
    assert!(save_text(&path, &source.revision, "stale").is_err());
}

#[test]
fn resolves_only_files_inside_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("code.ts");
    fs::write(&path, "").unwrap();
    assert_eq!(
        resolve(root.path(), "code.ts").unwrap(),
        path.canonicalize().unwrap()
    );
    assert!(resolve(root.path(), "../outside.ts").is_err());
    assert!(resolve(root.path(), ".git/config").is_err());
}

#[test]
fn workspace_commands_open_save_and_find_the_same_file() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().to_str().unwrap().to_owned();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/app.ts"), "first\nsecond\n").unwrap();
    tauri::async_runtime::block_on(async {
        let files = code_entries(cwd.clone(), "src".into()).await.unwrap();
        assert_eq!(files[0].path, "src/app.ts");

        let FileView::Text(source) = code_read(cwd.clone(), files[0].path.clone(), None)
            .await
            .unwrap()
        else {
            panic!("expected a text file");
        };
        let revision = code_save(
            cwd.clone(),
            files[0].path.clone(),
            source.revision,
            "first\nupdated\n".into(),
        )
        .await
        .unwrap();
        assert!(!revision.is_empty());

        let matches = code_search(cwd.clone(), "UPDATED".into(), true)
            .await
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, files[0].path);
        assert_eq!(matches[0].line, Some(2));
        let matches = code_search(cwd, "app.ts".into(), false).await.unwrap();
        assert_eq!(matches[0].path, files[0].path);
    });
}
#[test]
fn creates_new_files_without_overwriting_or_escaping_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    create_file(root.path(), "src", "new.ts", "").unwrap();
    let file = root.path().join("src/new.ts");
    assert_eq!(fs::read(&file).unwrap(), b"");
    fs::write(&file, "keep").unwrap();
    assert!(create_file(root.path(), "src", "new.ts", "").is_err());
    assert!(create_file(root.path(), "..", "escape.ts", "").is_err());
    assert!(create_file(root.path(), "", "../escape.ts", "").is_err());
    assert!(create_file(root.path(), "", ".git", "").is_err());
    assert!(create_file(root.path(), "", "NUL.txt", "").is_err());
    assert_eq!(fs::read(&file).unwrap(), b"keep");
    assert!(create_file(root.path(), "src", "new.ts", "replacement").is_err());
    assert_eq!(fs::read(&file).unwrap(), b"keep");
    create_file(root.path(), "src", "untitled.ts", "draft contents\n").unwrap();
    assert_eq!(
        fs::read(root.path().join("src/untitled.ts")).unwrap(),
        b"draft contents\n"
    );
}

#[test]
fn git_baseline_distinguishes_ignored_files_from_additions() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().to_str().unwrap();
    git(cwd, &["init", "-q"]).unwrap();
    fs::write(root.path().join(".gitignore"), "dist/\n").unwrap();
    fs::write(root.path().join("existing.txt"), "original\n").unwrap();
    git(cwd, &["add", ".gitignore", "existing.txt"]).unwrap();
    git(
        cwd,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "initial",
        ],
    )
    .unwrap();

    fs::create_dir(root.path().join("dist")).unwrap();
    fs::write(root.path().join("dist/index.html"), "generated").unwrap();
    fs::write(root.path().join("new.txt"), "new").unwrap();
    assert_eq!(
        baseline(cwd, "existing.txt", None).as_deref(),
        Some("original\n")
    );
    assert_eq!(baseline(cwd, "new.txt", None).as_deref(), Some(""));
    assert_eq!(baseline(cwd, "dist/index.html", None), None);
    git(cwd, &["add", "-f", "dist/index.html"]).unwrap();
    assert_eq!(baseline(cwd, "dist/index.html", None).as_deref(), Some(""));
}

#[test]
fn trash_action_accepts_only_workspace_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("example.ts");
    fs::write(&path, "code").unwrap();
    assert!(trash_file(root.path(), "", |_| panic!("must reject directories")).is_err());
    assert!(
        trash_file(root.path(), "../outside.ts", |_| panic!(
            "must reject traversal"
        ))
        .is_err()
    );
    trash_file(root.path(), "example.ts", |file| {
        assert_eq!(file, path.canonicalize().unwrap());
        Ok(())
    })
    .unwrap();
}
