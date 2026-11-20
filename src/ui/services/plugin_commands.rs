//! 插件 slash 候选的只读快照与后台刷新。
//!
//! UI 线程只消费内存和本地结果队列；插件发现限频运行在独立任务中，不进入按键或绘制路径。
//! 请求绑定实际 cwd 与代次，离开命令界面或切换项目即取消；错误和超时退回静态命令。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::commands::system::PluginCommandCandidate;
use crate::ui::state::{ChatState, PaletteMode};

const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

struct Completion {
    generation: u64,
    root: PathBuf,
    candidates: Vec<PluginCommandCandidate>,
}

pub(crate) struct PluginCommandCatalog {
    candidates: Vec<PluginCommandCandidate>,
    root: Option<PathBuf>,
    generation: u64,
    last_scan: Option<Instant>,
    task: Option<tokio::task::JoinHandle<()>>,
    sender: tokio::sync::mpsc::UnboundedSender<Completion>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<Completion>,
    pub(crate) inline_selection_lost: bool,
    pub(crate) palette_selection_lost: bool,
}

impl Default for PluginCommandCatalog {
    fn default() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            candidates: Vec::new(),
            root: None,
            generation: 0,
            last_scan: None,
            task: None,
            sender,
            receiver,
            inline_selection_lost: false,
            palette_selection_lost: false,
        }
    }
}

impl Drop for PluginCommandCatalog {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl PluginCommandCatalog {
    pub(crate) fn candidates(&self) -> &[PluginCommandCandidate] {
        &self.candidates
    }

    fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.generation = self.generation.wrapping_add(1);
    }

    fn update(
        &mut self,
        root: Option<PathBuf>,
        now: Instant,
    ) -> Option<Vec<PluginCommandCandidate>> {
        let mut next = None;
        if self.root != root {
            self.cancel();
            self.root = root;
            self.last_scan = None;
            next = Some(Vec::new());
        }
        if self.task.as_ref().is_some_and(|task| task.is_finished()) {
            self.task = None;
            // 任务异常结束没有回包时也不能让陈旧候选永久留下。
            next = Some(Vec::new());
        }
        while let Ok(completion) = self.receiver.try_recv() {
            if self.root.as_ref() == Some(&completion.root)
                && self.generation == completion.generation
            {
                self.task = None;
                next = Some(completion.candidates);
            }
        }
        self.refresh(now);
        next.filter(|candidates| self.candidates != *candidates)
    }

    #[cfg(test)]
    pub(crate) fn seed_for_tests(&mut self, candidates: Vec<PluginCommandCandidate>) {
        self.root = std::env::current_dir().ok();
        self.candidates = candidates;
    }

    fn refresh(&mut self, now: Instant) {
        if self.task.is_some()
            || self
                .last_scan
                .is_some_and(|last| now.duration_since(last) < REFRESH_INTERVAL)
        {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        self.last_scan = Some(now);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let sender = self.sender.clone();
        self.task = Some(runtime.spawn(async move {
            let candidates = match tokio::time::timeout(
                REFRESH_TIMEOUT,
                crate::core::plugins::discover_plugin_commands(&root),
            )
            .await
            {
                Ok(Ok(commands)) => commands
                    .into_iter()
                    .map(|command| PluginCommandCandidate {
                        name: command.name,
                        description: command.description,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let _ = sender.send(Completion {
                generation,
                root,
                candidates,
            });
        }));
    }
}

/// 由事件循环调用；不会等待插件发现，也不在渲染函数里推进刷新状态。
pub(crate) fn poll(state: &mut ChatState) -> bool {
    let command_palette =
        state.is_palette_open() && matches!(state.palette_mode, PaletteMode::Command);
    let active = command_palette
        || (!state.is_palette_open() && state.show_command_hints && state.input.starts_with('/'));
    let root = active.then(|| std::env::current_dir().ok()).flatten();
    let Some(candidates) = state.plugin_commands.update(root, Instant::now()) else {
        return false;
    };
    apply_candidates(state, candidates);
    true
}

/// 接受候选前只核对进程 cwd，不读插件文件，也不应用后台结果。
pub(crate) fn invalidate_changed_root(state: &mut ChatState) {
    if state.plugin_commands.root.is_some()
        && state.plugin_commands.root != std::env::current_dir().ok()
    {
        state.plugin_commands.cancel();
        state.plugin_commands.root = None;
        state.plugin_commands.last_scan = None;
        apply_candidates(state, Vec::new());
    }
}

fn apply_candidates(state: &mut ChatState, candidates: Vec<PluginCommandCandidate>) {
    let command_palette =
        state.is_palette_open() && matches!(state.palette_mode, PaletteMode::Command);
    let previous_inline = state
        .command_hints
        .get(state.selected_hint)
        .and_then(|hint| hint.split_once(" - ").map(|(name, _)| name.to_string()));
    let previous_palette = command_palette
        .then(|| {
            crate::ui::components::palette::palette_display_items(
                &state.palette_mode,
                state,
                &state.palette_filter,
            )
            .get(state.selected_palette_index)
            .map(|item| item.id.clone())
        })
        .flatten();
    state.plugin_commands.candidates = candidates;
    // 面板只遮住提示，不清除它；隐藏时也要同步，否则关面板会露出旧候选。
    if state.show_command_hints {
        crate::ui::services::suggestions::refresh_command_hints(state);
    }
    if let Some(previous) = previous_inline {
        match state.command_hints.iter().position(|hint| {
            hint.split_once(" - ")
                .is_some_and(|(name, _)| name == previous)
        }) {
            Some(index) => state.selected_hint = index,
            None => state.plugin_commands.inline_selection_lost = true,
        }
    }
    if let Some(previous) = previous_palette {
        let items = crate::ui::components::palette::palette_display_items(
            &state.palette_mode,
            state,
            &state.palette_filter,
        );
        match items.iter().position(|item| item.id == previous) {
            Some(index) => state.selected_palette_index = index,
            None => state.plugin_commands.palette_selection_lost = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str) -> PluginCommandCandidate {
        PluginCommandCandidate {
            name: name.into(),
            description: "Fixture command".into(),
        }
    }

    #[test]
    fn catalog_discards_previous_project_and_generation_results() {
        let mut catalog = PluginCommandCatalog::default();
        let old = PathBuf::from("old-project");
        let new = PathBuf::from("new-project");
        catalog.root = Some(old.clone());
        catalog.generation = 4;
        catalog.candidates = vec![candidate("fixture-old")];
        catalog
            .sender
            .send(Completion {
                root: old,
                generation: 4,
                candidates: vec![candidate("fixture-late")],
            })
            .unwrap();
        assert_eq!(
            catalog.update(Some(new.clone()), Instant::now()),
            Some(Vec::new())
        );
        catalog.candidates.clear();
        catalog
            .sender
            .send(Completion {
                root: new.clone(),
                generation: 4,
                candidates: vec![candidate("fixture-obsolete")],
            })
            .unwrap();
        assert_eq!(catalog.update(Some(new.clone()), Instant::now()), None);
        catalog
            .sender
            .send(Completion {
                root: new.clone(),
                generation: catalog.generation,
                candidates: vec![candidate("fixture-current")],
            })
            .unwrap();
        assert_eq!(
            catalog.update(Some(new), Instant::now()),
            Some(vec![candidate("fixture-current")])
        );
    }

    #[test]
    fn catalog_clears_when_command_ui_closes() {
        let mut catalog = PluginCommandCatalog::default();
        catalog.root = Some(PathBuf::from("fixture-project"));
        catalog.candidates = vec![candidate("fixture-check")];
        assert_eq!(catalog.update(None, Instant::now()), Some(Vec::new()));
        assert!(catalog.root.is_none());
        assert!(catalog.task.is_none());
    }

    #[test]
    fn closing_command_palette_stops_discovery_despite_remembered_mode() {
        let mut state = ChatState::test_fixture();
        state.open_palette(PaletteMode::Command);
        state
            .plugin_commands
            .seed_for_tests(vec![candidate("fixture-check")]);
        state.close_palette();
        poll(&mut state);
        assert!(state.plugin_commands.candidates().is_empty());
        assert!(state.plugin_commands.root.is_none());
        assert!(state.plugin_commands.task.is_none());
    }

    #[test]
    fn switching_project_before_accepting_drops_inline_candidates() {
        let _project = crate::ui::test_support::TestProject::new();
        for use_tab in [false, true] {
            let mut state = ChatState::test_fixture();
            state.set_input_text("/fixture-");
            crate::ui::services::suggestions::on_input_changed(&mut state);
            state.plugin_commands.root = Some(PathBuf::from("different-project"));
            apply_candidates(&mut state, vec![candidate("fixture-check")]);
            assert!(if use_tab {
                crate::ui::services::suggestions::handle_tab(&mut state)
            } else {
                crate::ui::services::suggestions::handle_enter(&mut state)
            });
            assert_eq!(state.input, "/fixture-");
            assert!(state.command_hints.is_empty());
        }
    }

    #[test]
    fn refresh_preserves_inline_selection_by_name_and_blocks_removed_selection() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("/fixture-");
        crate::ui::services::suggestions::on_input_changed(&mut state);
        apply_candidates(
            &mut state,
            vec![candidate("fixture-b"), candidate("fixture-c")],
        );
        state.selected_hint = 1;
        apply_candidates(
            &mut state,
            vec![
                candidate("fixture-a"),
                candidate("fixture-b"),
                candidate("fixture-c"),
            ],
        );
        assert!(state.command_hints[state.selected_hint].starts_with("/fixture-c - "));
        apply_candidates(
            &mut state,
            vec![candidate("fixture-a"), candidate("fixture-b")],
        );
        assert!(state.plugin_commands.inline_selection_lost);
        let previous = state.input.clone();
        assert!(crate::ui::services::suggestions::handle_enter(&mut state));
        assert_eq!(
            state.input, previous,
            "a disappearing selection must not accept its replacement"
        );
    }

    #[test]
    fn refresh_preserves_palette_selection_by_id_and_marks_removed_selection() {
        let mut state = ChatState::test_fixture();
        state.open_palette(PaletteMode::Command);
        state.palette_filter = "fixture-".into();
        apply_candidates(
            &mut state,
            vec![candidate("fixture-b"), candidate("fixture-c")],
        );
        state.selected_palette_index = 1;
        apply_candidates(
            &mut state,
            vec![
                candidate("fixture-a"),
                candidate("fixture-b"),
                candidate("fixture-c"),
            ],
        );
        let items = crate::ui::components::palette::palette_display_items(
            &state.palette_mode,
            &state,
            &state.palette_filter,
        );
        assert_eq!(items[state.selected_palette_index].label, "/fixture-c");
        apply_candidates(&mut state, vec![candidate("fixture-a")]);
        assert!(state.plugin_commands.palette_selection_lost);
    }

    struct DiskFixture {
        project: crate::ui::test_support::TestProject,
        _global: tempfile::TempDir,
        previous_config: Option<std::ffi::OsString>,
    }

    impl DiskFixture {
        fn new() -> Self {
            let project = crate::ui::test_support::TestProject::new();
            let global = tempfile::tempdir().unwrap();
            let previous_config = std::env::var_os("STAR_CONFIG_DIR");
            std::env::set_var("STAR_CONFIG_DIR", global.path());
            Self {
                project,
                _global: global,
                previous_config,
            }
        }

        async fn register(&self) {
            use crate::core::plugins::{self, PluginEntry, PluginManifest};
            let root = plugins::plugins_dir(self.project.path()).join("fixture-plugin");
            std::fs::create_dir_all(root.join(".star-plugin")).unwrap();
            std::fs::write(
                root.join(".star-plugin/plugin.json"),
                serde_json::json!({
                    "name": "fixture-plugin",
                    "commands": [{"name": "fixture-check", "description": "Fixture metadata",
                        "command": "echo unexpected > completion-executed.marker"}]
                })
                .to_string(),
            )
            .unwrap();
            plugins::save_manifest(
                self.project.path(),
                &PluginManifest {
                    plugins: vec![PluginEntry {
                        name: "fixture-plugin".into(),
                        source: "fixture".into(),
                        install_type: "local".into(),
                        installed_at: 0,
                        enabled: true,
                        scope: plugins::SCOPE_PROJECT.into(),
                    }],
                },
            )
            .await
            .unwrap();
        }
    }

    impl Drop for DiskFixture {
        fn drop(&mut self) {
            match self.previous_config.take() {
                Some(value) => std::env::set_var("STAR_CONFIG_DIR", value),
                None => std::env::remove_var("STAR_CONFIG_DIR"),
            }
        }
    }

    async fn rescan(state: &mut ChatState) {
        state.plugin_commands.last_scan = None;
        poll(state);
        tokio::time::timeout(Duration::from_secs(10), async {
            while state
                .plugin_commands
                .task
                .as_ref()
                .is_some_and(|task| !task.is_finished())
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("candidate scan finishes");
        poll(state);
    }

    #[tokio::test]
    async fn disk_discovery_refreshes_enable_disable_remove_and_registry_errors() {
        use crate::core::plugins;
        let fixture = DiskFixture::new();
        fixture.register().await;
        let mut state = ChatState::test_fixture();
        state.set_input_text("/fixture-");
        crate::ui::services::suggestions::on_input_changed(&mut state);
        rescan(&mut state).await;
        assert!(state
            .command_hints
            .iter()
            .any(|hint| hint.starts_with("/fixture-check - ")));
        let root = plugins::plugins_dir(fixture.project.path()).join("fixture-plugin");
        assert!(!root.join("completion-executed.marker").exists());
        assert!(!fixture
            .project
            .path()
            .join("completion-executed.marker")
            .exists());
        let mut manifest = plugins::load_manifest(fixture.project.path())
            .await
            .unwrap();
        manifest.plugins[0].enabled = false;
        plugins::save_manifest(fixture.project.path(), &manifest)
            .await
            .unwrap();
        rescan(&mut state).await;
        assert!(state.command_hints.is_empty());
        manifest.plugins[0].enabled = true;
        plugins::save_manifest(fixture.project.path(), &manifest)
            .await
            .unwrap();
        rescan(&mut state).await;
        assert!(!state.command_hints.is_empty());
        // 外部直接改runtime文件，不走任何进程内失效通知。
        std::fs::write(root.join(".star-plugin/plugin.json"), "{broken").unwrap();
        rescan(&mut state).await;
        assert!(state.command_hints.is_empty());
        fixture.register().await;
        rescan(&mut state).await;
        assert!(!state.command_hints.is_empty());
        manifest.plugins.clear();
        plugins::save_manifest(fixture.project.path(), &manifest)
            .await
            .unwrap();
        rescan(&mut state).await;
        assert!(state.command_hints.is_empty());
        std::fs::write(plugins::manifest_path(fixture.project.path()), "{broken").unwrap();
        state.set_input_text("/");
        crate::ui::services::suggestions::on_input_changed(&mut state);
        rescan(&mut state).await;
        assert!(state.plugin_commands.candidates().is_empty());
        assert!(state
            .command_hints
            .iter()
            .any(|hint| hint.starts_with("/help - ")));
    }

    #[tokio::test]
    async fn returning_from_main_palette_does_not_accept_invalidated_plugin_hint() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for accept in [KeyCode::Enter, KeyCode::Tab] {
            let mut state = ChatState::test_fixture();
            state
                .plugin_commands
                .seed_for_tests(vec![candidate("fixture-check")]);
            state.set_input_text("/fixture-");
            crate::ui::services::suggestions::on_input_changed(&mut state);
            assert_eq!(state.command_hints.len(), 1);
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            for key in [
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                KeyEvent::new(accept, KeyModifiers::NONE),
            ] {
                crate::ui::events::input::handle_key_event(&mut state, key, &tx, None)
                    .await
                    .unwrap();
                if key.code == KeyCode::Char('p') {
                    assert!(state.is_palette_open());
                    assert!(matches!(state.palette_mode, PaletteMode::Main));
                    assert!(poll(&mut state));
                    assert!(state.plugin_commands.candidates().is_empty());
                }
            }
            assert_eq!(state.input, "/fixture-", "{accept:?}");
            assert_eq!(state.textarea.lines().join("\n"), state.input);
            assert!(state.command_hints.is_empty());
            assert!(rx.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn disk_candidate_fills_command_palette_without_executing_payload() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let fixture = DiskFixture::new();
        fixture.register().await;
        let mut state = ChatState::test_fixture();
        state.open_palette(PaletteMode::Command);
        state.palette_filter = "fixture-check".into();
        rescan(&mut state).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        crate::ui::events::input::handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "/fixture-check ");
        assert_eq!(
            state.textarea.lines().join("\n"),
            state.input,
            "the command palette must fill the actual editable prompt, not only its cache"
        );
        assert!(!state.is_palette_open());
        assert!(rx.try_recv().is_err());
        assert!(!crate::core::plugins::plugins_dir(fixture.project.path())
            .join("fixture-plugin/completion-executed.marker")
            .exists());
        assert!(!fixture
            .project
            .path()
            .join("completion-executed.marker")
            .exists());
    }

    #[tokio::test]
    async fn removed_palette_selection_does_not_accept_a_different_command() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = ChatState::test_fixture();
        state.open_palette(PaletteMode::Command);
        state.palette_filter = "fixture-".into();
        apply_candidates(
            &mut state,
            vec![candidate("fixture-b"), candidate("fixture-c")],
        );
        state.selected_palette_index = 1;
        apply_candidates(&mut state, vec![candidate("fixture-a")]);
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        crate::ui::events::input::handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(state.input.is_empty());
        assert!(matches!(state.palette_mode, PaletteMode::Command));
    }

    #[tokio::test]
    async fn switching_project_before_palette_enter_cannot_fill_old_command() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _project = crate::ui::test_support::TestProject::new();
        let mut state = ChatState::test_fixture();
        state.open_palette(PaletteMode::Command);
        state.palette_filter = "fixture-".into();
        state.plugin_commands.root = Some(PathBuf::from("different-project"));
        apply_candidates(&mut state, vec![candidate("fixture-check")]);
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        crate::ui::events::input::handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(state.input.is_empty());
        assert!(state.is_palette_open());
        assert!(state.plugin_commands.candidates().is_empty());
    }

    #[tokio::test]
    async fn catalog_limits_refresh_and_cancels_outstanding_scan() {
        let fixture = DiskFixture::new();
        let mut catalog = PluginCommandCatalog::default();
        let now = Instant::now();
        catalog.update(Some(fixture.project.path().to_path_buf()), now);
        assert!(catalog.task.is_some());
        let generation = catalog.generation;
        catalog.refresh(now);
        assert_eq!(
            catalog.generation, generation,
            "only one task may be outstanding"
        );
        catalog.cancel();
        catalog.refresh(now + Duration::from_millis(500));
        assert!(
            catalog.task.is_none(),
            "refresh interval must not be bypassed"
        );
        catalog.refresh(now + REFRESH_INTERVAL);
        assert!(catalog.task.is_some());
        catalog.update(None, now + REFRESH_INTERVAL);
        assert!(catalog.task.is_none());
    }
}
