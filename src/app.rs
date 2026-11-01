use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gpui_kit::assets::IconName;
use gpui_kit::base::{ElementExt as _, StyledExt as _};
use gpui_kit::component::{
    ActiveTheme, Icon, Selectable as _, Sizable, Theme,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
    text::TextView,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use uuid::Uuid;

use crate::model::{
    BoardNode, Camera, MIND_FONT_SIZE, NodeKind, NoteDoc, TextFormat, TreeEntry, board_path,
    create_folder, create_note, ensure_data_root, estimate_mind_lines, mind_height_for_lines,
    mind_text_width, note_title, read_clipboard_image, read_clipboard_text, sanitize_name,
    scan_tree, validate_note,
};
use crate::settings::{
    AppSettings, Language, THEME_PRESETS, ThemePreset, accent_preview, apply_theme, optional_color,
    path_is_usable, set_notes_root_default,
};

pub(crate) const CUSTOM_TITLEBAR: bool = cfg!(all(target_os = "windows", not(debug_assertions)));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreateMode {
    Folder,
    Markdown,
    Plain,
}

impl CreateMode {
    fn is_note(self) -> bool {
        !matches!(self, Self::Folder)
    }

    fn format(self) -> TextFormat {
        match self {
            Self::Plain => TextFormat::Plain,
            _ => TextFormat::Markdown,
        }
    }
}

#[derive(Clone, Debug)]
struct CreateDialog {
    mode: CreateMode,
    parent: PathBuf,
}

#[derive(Clone, Debug)]
struct RenameDialog {
    path: PathBuf,
    is_dir: bool,
}

#[derive(Clone, Debug)]
struct DeleteDialog {
    path: PathBuf,
    is_dir: bool,
}

#[derive(Clone, Copy, Debug)]
enum EditTarget {
    Note,
    Text(Uuid),
    Mind(Uuid),
}

#[derive(Clone, Copy, Debug)]
enum NodeMenuKind {
    Note,
    Text(TextFormat),
    Mind,
    Image,
}

#[derive(Clone, Copy, Debug)]
struct EditorModal {
    target: EditTarget,
    format: TextFormat,
}

#[derive(Clone, Copy, Debug, Default)]
enum Interaction {
    #[default]
    None,
    Panning {
        start_x: f32,
        start_y: f32,
        pan_x: f32,
        pan_y: f32,
    },
    Dragging {
        id: Uuid,
        offset_x: f32,
        offset_y: f32,
    },
    BoxSelecting {
        start_x: f32,
        start_y: f32,
        current_x: f32,
        current_y: f32,
    },
    Resizing {
        id: Uuid,
        start_x: f32,
        start_y: f32,
        start_w: f32,
        start_h: f32,
    },
}

#[derive(Clone, Debug)]
struct GroupDrag {
    anchor_x: f32,
    anchor_y: f32,
    nodes: Vec<(Uuid, f32, f32)>,
}

#[derive(Clone, Copy, Debug)]
struct EditorResize {
    start_x: f32,
    start_y: f32,
    start_width: f32,
    start_height: f32,
}

pub struct WhiteboardApp {
    root: PathBuf,
    entries: Vec<TreeEntry>,
    expanded: HashSet<PathBuf>,
    selected_dir: Option<PathBuf>,
    selected_note: Option<PathBuf>,
    selected_tree: Option<PathBuf>,
    note: Option<NoteDoc>,
    selected_node: Option<Uuid>,
    multi_selected: HashSet<Uuid>,
    group_drag: Option<GroupDrag>,
    rotated_svg_cache: RefCell<HashMap<u64, Arc<Image>>>,
    board_bounds: Option<Bounds<Pixels>>,
    interaction: Interaction,
    settings: AppSettings,
    settings_open: bool,
    sidebar_collapsed: bool,
    sidebar_layout_id: u64,
    board_focus: FocusHandle,
    create_input: Entity<InputState>,
    rename_input: Entity<InputState>,
    title_input: Entity<InputState>,
    body_input: Entity<TextareaState>,
    accent_input: Entity<InputState>,
    background_input: Entity<InputState>,
    foreground_input: Entity<InputState>,
    sidebar_input: Entity<InputState>,
    border_input: Entity<InputState>,
    create_dialog: Option<CreateDialog>,
    rename_dialog: Option<RenameDialog>,
    delete_dialog: Option<DeleteDialog>,
    editor: Option<EditorModal>,
    editor_width: f32,
    editor_height: f32,
    editor_resize: Option<EditorResize>,
    status: String,
    _subscriptions: Vec<Subscription>,
}

impl WhiteboardApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, settings: AppSettings) -> Self {
        let language = settings.language;
        let root = settings.effective_notes_root();
        let mut status = String::new();
        if let Err(error) = ensure_data_root(&root, language) {
            status = if language.is_zh() {
                format!("初始化失败: {error:#}")
            } else {
                format!("Initialization failed: {error:#}")
            };
        }

        let expanded = HashSet::new();
        let entries = scan_tree(&root, &expanded);

        let create_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(language.pick(
                "输入名称，留空则使用默认名称",
                "Enter a name, or leave blank for the default",
            ))
        });
        let rename_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(language.pick("输入新名称", "Enter a new name"))
        });
        let title_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(language.pick("输入标题", "Enter a title"))
        });
        let body_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(18)
                .soft_wrap(true)
                .placeholder(language.pick(
                    "在这里输入 Markdown 或纯文本内容……",
                    "Write Markdown or plain text here...",
                ))
        });
        let accent_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.theme.accent.clone().unwrap_or_default())
                .placeholder("#3B82F6")
        });
        let background_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.theme.background.clone().unwrap_or_default())
                .placeholder("#FFFFFF")
        });
        let foreground_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.theme.foreground.clone().unwrap_or_default())
                .placeholder("#111111")
        });
        let sidebar_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.theme.sidebar.clone().unwrap_or_default())
                .placeholder("#F5F5F5")
        });
        let border_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.theme.border.clone().unwrap_or_default())
                .placeholder("#E5E5E5")
        });

        let mut this = Self {
            root: root.clone(),
            entries,
            expanded,
            selected_dir: None,
            selected_note: None,
            selected_tree: None,
            note: None,
            selected_node: None,
            multi_selected: HashSet::new(),
            group_drag: None,
            rotated_svg_cache: RefCell::new(HashMap::new()),
            board_bounds: None,
            interaction: Interaction::None,
            settings,
            settings_open: false,
            sidebar_collapsed: false,
            sidebar_layout_id: 0,
            board_focus: cx.focus_handle(),
            create_input: create_input.clone(),
            rename_input: rename_input.clone(),
            title_input: title_input.clone(),
            body_input: body_input.clone(),
            accent_input: accent_input.clone(),
            background_input: background_input.clone(),
            foreground_input: foreground_input.clone(),
            sidebar_input: sidebar_input.clone(),
            border_input: border_input.clone(),
            create_dialog: None,
            rename_dialog: None,
            delete_dialog: None,
            editor: None,
            editor_width: 760.0,
            editor_height: 600.0,
            editor_resize: None,
            status,
            _subscriptions: Vec::new(),
        };

        let create_subscription =
            cx.subscribe_in(&create_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.confirm_create(window, cx);
                }
            });
        let title_subscription =
            cx.subscribe_in(&title_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_editor(window, cx);
                }
            });
        let rename_subscription =
            cx.subscribe_in(&rename_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.confirm_rename(window, cx);
                }
            });
        this._subscriptions.push(create_subscription);
        this._subscriptions.push(title_subscription);
        this._subscriptions.push(rename_subscription);

        this.refresh_tree();
        for entry in this
            .entries
            .iter()
            .filter(|entry| entry.is_dir && entry.depth == 0)
        {
            this.expanded.insert(entry.path.clone());
        }
        this.refresh_tree();

        if let Some(first_note) = this
            .entries
            .iter()
            .find(|entry| !entry.is_dir)
            .map(|entry| entry.path.clone())
        {
            this.open_note(&first_note, window, cx);
        }

        if this.note.is_none() && this.status.is_empty() {
            this.status = language
                .pick("新建一篇笔记开始使用", "Create a note to get started")
                .to_string();
        }

        this
    }

    fn tr(&self, zh: &'static str, en: &'static str) -> &'static str {
        self.settings.language.pick(zh, en)
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.sidebar_layout_id = self.sidebar_layout_id.wrapping_add(1);
        cx.notify();
    }

    fn collapse_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = true;
        self.sidebar_layout_id = self.sidebar_layout_id.wrapping_add(1);
        cx.notify();
    }

    fn select_single_node(&mut self, id: Uuid) {
        self.multi_selected.clear();
        self.group_drag = None;
        self.selected_node = Some(id);
    }

    fn select_node_from_pointer(&mut self, id: Uuid) {
        if !self.multi_selected.contains(&id) {
            self.multi_selected.clear();
        }
        self.group_drag = None;
        self.selected_node = Some(id);
    }

    fn clear_node_selection(&mut self) {
        self.multi_selected.clear();
        self.group_drag = None;
        self.selected_node = None;
    }

    fn is_zh(&self) -> bool {
        self.settings.language.is_zh()
    }

    fn update_input_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.settings.language;
        self.create_input.update(cx, |state, cx| {
            state.set_placeholder(
                language.pick(
                    "输入名称，留空则使用默认名称",
                    "Enter a name, or leave blank for the default",
                ),
                window,
                cx,
            );
        });
        self.rename_input.update(cx, |state, cx| {
            state.set_placeholder(language.pick("输入新名称", "Enter a new name"), window, cx);
        });
        self.title_input.update(cx, |state, cx| {
            state.set_placeholder(language.pick("输入标题", "Enter a title"), window, cx);
        });
        self.body_input.update(cx, |state, cx| {
            state.set_placeholder(
                language.pick(
                    "在这里输入 Markdown 或纯文本内容……",
                    "Write Markdown or plain text here...",
                ),
                window,
                cx,
            );
        });
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.language = language;
        let _ = self.settings.save();
        self.update_input_placeholders(window, cx);
        window.set_window_title(language.pick("白板笔记", "Whiteboard Notes"));
        self.status = language.pick("语言已切换", "Language changed").to_string();
        cx.notify();
    }

    fn set_theme_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.settings.theme.dark = dark;
        apply_theme(&self.settings.theme, cx);
        self.rotated_svg_cache.borrow_mut().clear();
        let _ = self.settings.save();
        self.status = self.tr("主题已更新", "Theme updated").to_string();
        cx.notify();
    }

    fn sync_theme_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.settings.theme.clone();
        self.accent_input.update(cx, |state, cx| {
            state.set_value(theme.accent.unwrap_or_default(), window, cx);
        });
        self.background_input.update(cx, |state, cx| {
            state.set_value(theme.background.unwrap_or_default(), window, cx);
        });
        self.foreground_input.update(cx, |state, cx| {
            state.set_value(theme.foreground.unwrap_or_default(), window, cx);
        });
        self.sidebar_input.update(cx, |state, cx| {
            state.set_value(theme.sidebar.unwrap_or_default(), window, cx);
        });
        self.border_input.update(cx, |state, cx| {
            state.set_value(theme.border.unwrap_or_default(), window, cx);
        });
    }

    fn apply_theme_preset(
        &mut self,
        preset: &ThemePreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.theme.apply_preset(preset);
        apply_theme(&self.settings.theme, cx);
        self.rotated_svg_cache.borrow_mut().clear();
        let _ = self.settings.save();
        self.sync_theme_inputs(window, cx);
        self.status = if self.is_zh() {
            format!("已应用主题：{}", preset.name)
        } else {
            format!("Applied theme: {}", preset.name)
        };
        cx.notify();
    }

    fn apply_custom_theme(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let accent = self.accent_input.read(cx).value().to_string();
        let background = self.background_input.read(cx).value().to_string();
        let foreground = self.foreground_input.read(cx).value().to_string();
        let sidebar = self.sidebar_input.read(cx).value().to_string();
        let border = self.border_input.read(cx).value().to_string();

        let parsed = [
            ("Accent", optional_color(&accent)),
            ("Background", optional_color(&background)),
            ("Foreground", optional_color(&foreground)),
            ("Sidebar", optional_color(&sidebar)),
            ("Border", optional_color(&border)),
        ];
        for (name, result) in &parsed {
            if let Err(error) = result {
                self.status = if self.is_zh() {
                    format!("{name} 颜色格式无效: {error:#}")
                } else {
                    format!("Invalid {name} color: {error:#}")
                };
                cx.notify();
                return;
            }
        }

        self.settings.theme.accent = optional_color(&accent).ok().flatten();
        self.settings.theme.background = optional_color(&background).ok().flatten();
        self.settings.theme.foreground = optional_color(&foreground).ok().flatten();
        self.settings.theme.sidebar = optional_color(&sidebar).ok().flatten();
        self.settings.theme.border = optional_color(&border).ok().flatten();

        apply_theme(&self.settings.theme, cx);
        self.rotated_svg_cache.borrow_mut().clear();
        let _ = self.settings.save();
        self.status = self
            .tr("自定义主题已应用", "Custom theme applied")
            .to_string();
        cx.notify();
    }

    fn reset_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.theme.dark = false;
        self.settings.theme.accent = None;
        self.settings.theme.background = None;
        self.settings.theme.foreground = None;
        self.settings.theme.sidebar = None;
        self.settings.theme.border = None;
        apply_theme(&self.settings.theme, cx);
        self.rotated_svg_cache.borrow_mut().clear();
        let _ = self.settings.save();
        self.sync_theme_inputs(window, cx);
        self.status = self
            .tr("主题已恢复默认", "Theme reset to default")
            .to_string();
        cx.notify();
    }

    fn set_notes_root(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path == self.root {
            return;
        }
        if !path_is_usable(&path) {
            self.status = if self.is_zh() {
                format!("无法使用该目录: {}", path.display())
            } else {
                format!("Cannot use this folder: {}", path.display())
            };
            cx.notify();
            return;
        }

        self.save_current();
        self.settings.notes_root = Some(path.clone());
        let _ = self.settings.save();
        self.root = path;
        self.selected_dir = None;
        self.selected_note = None;
        self.selected_tree = None;
        self.note = None;
        self.clear_node_selection();
        self.expanded.clear();
        self.interaction = Interaction::None;

        if let Err(error) = ensure_data_root(&self.root, self.settings.language) {
            self.status = if self.is_zh() {
                format!("切换目录失败: {error:#}")
            } else {
                format!("Failed to switch folder: {error:#}")
            };
            cx.notify();
            return;
        }

        self.refresh_tree();
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.is_dir && entry.depth == 0)
        {
            self.expanded.insert(entry.path.clone());
        }
        self.refresh_tree();

        if let Some(first_note) = self
            .entries
            .iter()
            .find(|entry| !entry.is_dir)
            .map(|entry| entry.path.clone())
        {
            self.open_note(&first_note, window, cx);
        }

        self.status = self
            .tr("保存路径已更新", "Notes folder updated")
            .to_string();
        cx.notify();
    }

    fn choose_notes_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.tr("选择笔记保存目录", "Choose notes folder").into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(path) = paths.into_iter().next()
            {
                _ = view.update_in(cx, |this, window, cx| {
                    this.set_notes_root(path, window, cx);
                });
            }
        })
        .detach();
    }

    fn refresh_tree(&mut self) {
        self.entries = scan_tree(&self.root, &self.expanded);
        let notes = self.entries.iter().filter(|entry| !entry.is_dir).count();
        let folders = self.entries.iter().filter(|entry| entry.is_dir).count();
        self.status = if self.is_zh() {
            format!("{notes} 篇笔记 · {folders} 个目录")
        } else {
            let note_word = if notes == 1 { "note" } else { "notes" };
            let folder_word = if folders == 1 { "folder" } else { "folders" };
            format!("{notes} {note_word} · {folders} {folder_word}")
        };
    }

    fn save_current(&mut self) {
        let Some(note) = &mut self.note else {
            return;
        };
        note.sync_note_content();
        if let Err(error) = note.save_content() {
            self.status = if self.is_zh() {
                format!("保存失败: {error:#}")
            } else {
                format!("Save failed: {error:#}")
            };
            return;
        }
        if let Err(error) = note.save_board() {
            self.status = if self.is_zh() {
                format!("保存白板失败: {error:#}")
            } else {
                format!("Failed to save board: {error:#}")
            };
        }
    }

    fn open_note(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if !validate_note(path) {
            self.status = self
                .tr(
                    "请选择 Markdown 或纯文本笔记",
                    "Select a Markdown or plain-text note",
                )
                .to_string();
            cx.notify();
            return;
        }

        if self.selected_note.as_deref() == Some(path) {
            self.board_focus.focus(window, cx);
            return;
        }

        self.save_current();

        match NoteDoc::load(path) {
            Ok(mut note) => {
                let font_family = cx.theme().font_family.clone();
                if fit_mind_node_heights(&mut note.board.nodes, window, &font_family) {
                    resolve_mind_overlaps(&mut note.board.nodes);
                    let _ = note.save_board();
                }
                let should_fit = self.should_fit_board(&note);
                self.selected_note = Some(note.path.clone());
                self.selected_tree = Some(note.path.clone());
                self.selected_dir = None;
                self.note = Some(note);
                self.select_single_node(self.note.as_ref().unwrap().note_id);
                self.interaction = Interaction::None;

                if should_fit {
                    let viewport = window.viewport_size();
                    let width = self
                        .board_bounds
                        .map(|bounds| f32::from(bounds.size.width))
                        .unwrap_or_else(|| (f32::from(viewport.width) - 320.0).max(320.0));
                    let height = self
                        .board_bounds
                        .map(|bounds| f32::from(bounds.size.height))
                        .unwrap_or_else(|| (f32::from(viewport.height) - 120.0).max(260.0));
                    self.fit_view_to_size(width, height);
                    if let Some(note) = &self.note {
                        let _ = note.save_board();
                    }
                }

                self.status = self.tr("已打开笔记", "Note opened").to_string();
                if !self.settings_open {
                    self.board_focus.focus(window, cx);
                }
            }
            Err(error) => {
                self.status = if self.is_zh() {
                    format!("打开失败: {error:#}")
                } else {
                    format!("Failed to open note: {error:#}")
                };
            }
        }

        cx.notify();
    }

    fn open_create_dialog(
        &mut self,
        mode: CreateMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = self
            .selected_dir
            .as_ref()
            .filter(|directory| directory.is_dir())
            .cloned()
            .unwrap_or_else(|| self.root.clone());
        self.create_dialog = Some(CreateDialog { mode, parent });
        self.create_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn confirm_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.create_dialog.take() else {
            return;
        };
        let name = self.create_input.read(cx).value().to_string();

        let result = if dialog.mode.is_note() {
            create_note(
                &dialog.parent,
                &name,
                dialog.mode.format(),
                self.settings.language,
            )
            .map(|note| note.path)
        } else {
            create_folder(&dialog.parent, &name, self.settings.language)
        };

        match result {
            Ok(path) => {
                self.expanded.insert(dialog.parent.clone());
                self.refresh_tree();
                if dialog.mode.is_note() {
                    self.selected_dir = None;
                    self.settings_open = false;
                    self.open_note(&path, window, cx);
                } else {
                    self.selected_tree = Some(path.clone());
                    self.selected_dir = Some(path);
                    self.status = self.tr("已创建目录", "Folder created").to_string();
                }
            }
            Err(error) => {
                self.status = if self.is_zh() {
                    format!("创建失败: {error:#}")
                } else {
                    format!("Create failed: {error:#}")
                };
            }
        }

        cx.notify();
    }

    fn open_rename_dialog(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path == self.root {
            self.status = self
                .tr("根目录不能重命名", "The root folder cannot be renamed")
                .to_string();
            cx.notify();
            return;
        }

        let is_dir = path.is_dir();
        if !is_dir && !validate_note(&path) {
            self.status = self
                .tr(
                    "只能重命名目录或笔记文件",
                    "Only folders and note files can be renamed",
                )
                .to_string();
            cx.notify();
            return;
        }

        let current_name = if is_dir {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(self.tr("目录", "Folder"))
        } else {
            path.file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or(self.tr("笔记", "Note"))
        }
        .to_string();

        self.rename_input.update(cx, |state, cx| {
            state.set_value(current_name, window, cx);
            state.select_all(window, cx);
        });
        self.rename_dialog = Some(RenameDialog { path, is_dir });
        let input = self.rename_input.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |state, cx| state.focus(window, cx));
        });
        cx.notify();
    }

    fn confirm_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.rename_dialog.take() else {
            return;
        };
        let old_path = dialog.path.clone();
        if old_path == self.root {
            self.status = self
                .tr("根目录不能重命名", "The root folder cannot be renamed")
                .to_string();
            cx.notify();
            return;
        }

        let raw_name = self.rename_input.read(cx).value().to_string();
        let raw_name = if !dialog.is_dir {
            let extension = old_path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("md");
            raw_name
                .strip_suffix(&format!(".{extension}"))
                .unwrap_or(&raw_name)
                .to_string()
        } else {
            raw_name
        };
        if raw_name.trim().is_empty() {
            self.status = self.tr("名称不能为空", "Name cannot be empty").to_string();
            self.rename_dialog = Some(dialog);
            let input = self.rename_input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |state, cx| state.focus(window, cx));
            });
            cx.notify();
            return;
        }
        let fallback = if dialog.is_dir {
            self.tr("未命名目录", "Untitled folder")
        } else {
            self.tr("未命名笔记", "Untitled note")
        };
        let name = sanitize_name(&raw_name, fallback);
        let Some(parent) = old_path.parent().map(Path::to_path_buf) else {
            self.status = self
                .tr("无法确定父目录", "Cannot determine the parent folder")
                .to_string();
            cx.notify();
            return;
        };

        let new_path = if dialog.is_dir {
            parent.join(&name)
        } else {
            let extension = old_path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("md");
            parent.join(format!("{name}.{extension}"))
        };

        if new_path == old_path {
            self.board_focus.focus(window, cx);
            cx.notify();
            return;
        }

        let old_board = board_path(&old_path);
        let new_board = board_path(&new_path);
        if new_path.exists() || (old_board.exists() && new_board.exists()) {
            self.status = self
                .tr(
                    "同名文件或目录已存在",
                    "A file or folder with this name already exists",
                )
                .to_string();
            self.rename_dialog = Some(dialog);
            let input = self.rename_input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |state, cx| state.focus(window, cx));
            });
            cx.notify();
            return;
        }

        if let Err(error) = fs::rename(&old_path, &new_path) {
            self.status = if self.is_zh() {
                format!("重命名失败: {error}")
            } else {
                format!("Rename failed: {error}")
            };
            self.rename_dialog = Some(dialog);
            let input = self.rename_input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |state, cx| state.focus(window, cx));
            });
            cx.notify();
            return;
        }

        if !dialog.is_dir
            && old_board.exists()
            && let Err(error) = fs::rename(&old_board, &new_board)
        {
            let rollback = fs::rename(&new_path, &old_path);
            self.status = match rollback {
                Ok(()) if self.is_zh() => {
                    format!("重命名白板数据失败，已回滚: {error}")
                }
                Ok(()) => format!("Failed to rename board data, rolled back: {error}"),
                Err(rollback_error) if self.is_zh() => {
                    format!("重命名白板数据失败: {error}；回滚失败: {rollback_error}")
                }
                Err(rollback_error) => {
                    format!(
                        "Failed to rename board data: {error}; rollback failed: {rollback_error}"
                    )
                }
            };
            self.rename_dialog = Some(dialog);
            let input = self.rename_input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |state, cx| state.focus(window, cx));
            });
            cx.notify();
            return;
        }

        self.expanded = self
            .expanded
            .iter()
            .map(|path| remap_path(path, &old_path, &new_path))
            .collect();
        self.selected_dir = self
            .selected_dir
            .as_ref()
            .map(|directory| remap_path(directory, &old_path, &new_path));
        if let Some(note_path) = &self.selected_note {
            let remapped = remap_path(note_path, &old_path, &new_path);
            self.selected_note = Some(remapped);
        }
        if let Some(note) = &mut self.note {
            let remapped = remap_path(&note.path, &old_path, &new_path);
            if remapped != note.path {
                note.path = remapped;
                note.title = note_title(&note.path);
            }
        }
        self.selected_tree = Some(new_path.clone());
        if let Some(parent) = new_path.parent() {
            self.expanded.insert(parent.to_path_buf());
        }
        if dialog.is_dir {
            self.expanded.insert(new_path.clone());
        }

        self.refresh_tree();
        self.status = if dialog.is_dir {
            self.tr("已重命名目录", "Folder renamed").to_string()
        } else {
            self.tr("已重命名笔记", "Note renamed").to_string()
        };
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn rename_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            self.rename_dialog = None;
            self.board_focus.focus(window, cx);
            cx.notify();
        } else if event.keystroke.key == "enter" {
            self.confirm_rename(window, cx);
        }
    }

    fn open_delete_dialog(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if path == self.root {
            self.status = self
                .tr("根目录不能删除", "The root folder cannot be deleted")
                .to_string();
            cx.notify();
            return;
        }
        let is_dir = path.is_dir();
        if !is_dir && !validate_note(&path) {
            self.status = self
                .tr(
                    "只能删除目录或笔记文件",
                    "Only folders and note files can be deleted",
                )
                .to_string();
            cx.notify();
            return;
        }
        self.delete_dialog = Some(DeleteDialog { path, is_dir });
        cx.notify();
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.delete_dialog.take() else {
            return;
        };
        let path = dialog.path.clone();
        if path == self.root {
            self.status = self
                .tr("根目录不能删除", "The root folder cannot be deleted")
                .to_string();
            cx.notify();
            return;
        }

        let affects_open_note = self
            .selected_note
            .as_ref()
            .is_some_and(|note| note.starts_with(&path))
            || self
                .note
                .as_ref()
                .is_some_and(|note| note.path.starts_with(&path));

        let result = if dialog.is_dir {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };

        match result {
            Ok(()) => {
                if !dialog.is_dir {
                    let _ = fs::remove_file(board_path(&path));
                }
                self.expanded
                    .retain(|expanded| !expanded.starts_with(&path));
                if self
                    .selected_dir
                    .as_ref()
                    .is_some_and(|directory| directory.starts_with(&path))
                {
                    self.selected_dir = None;
                }
                if self
                    .selected_tree
                    .as_ref()
                    .is_some_and(|selected| selected.starts_with(&path))
                {
                    self.selected_tree = None;
                }
                if affects_open_note {
                    self.note = None;
                    self.selected_note = None;
                    self.selected_node = None;
                    self.multi_selected.clear();
                    self.group_drag = None;
                }

                self.refresh_tree();
                if affects_open_note
                    && let Some(next_note) = self
                        .entries
                        .iter()
                        .find(|entry| !entry.is_dir)
                        .map(|entry| entry.path.clone())
                {
                    self.open_note(&next_note, window, cx);
                }
                self.status = if dialog.is_dir {
                    self.tr("已删除目录", "Folder deleted").to_string()
                } else {
                    self.tr("已删除笔记", "Note deleted").to_string()
                };
            }
            Err(error) => {
                self.status = if self.is_zh() {
                    format!("删除失败: {error}")
                } else {
                    format!("Delete failed: {error}")
                };
            }
        }

        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn delete_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            self.delete_dialog = None;
            self.board_focus.focus(window, cx);
            cx.notify();
        } else if event.keystroke.key == "enter" {
            self.confirm_delete(window, cx);
        }
    }

    fn open_editor_for_node(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = &self.note else {
            return;
        };
        self.editor_resize = None;
        let Some(node) = note.board.node(id) else {
            return;
        };

        match &node.kind {
            NodeKind::Note { content, format } => {
                let content = content.clone();
                let format = *format;
                self.editor = Some(EditorModal {
                    target: EditTarget::Note,
                    format,
                });
                self.body_input.update(cx, |state, cx| {
                    state.set_value(content, window, cx);
                });
                let input = self.body_input.clone();
                window.defer(cx, move |window, cx| {
                    input.update(cx, |state, cx| state.focus(window, cx));
                });
            }
            NodeKind::Text { content, format } => {
                let content = content.clone();
                let format = *format;
                self.editor = Some(EditorModal {
                    target: EditTarget::Text(id),
                    format,
                });
                self.body_input.update(cx, |state, cx| {
                    state.set_value(content, window, cx);
                });
                let input = self.body_input.clone();
                window.defer(cx, move |window, cx| {
                    input.update(cx, |state, cx| state.focus(window, cx));
                });
            }
            NodeKind::Mind { title, .. } => {
                let title = title.clone();
                self.editor = Some(EditorModal {
                    target: EditTarget::Mind(id),
                    format: TextFormat::Plain,
                });
                self.title_input.update(cx, |state, cx| {
                    state.set_value(title, window, cx);
                });
                let input = self.title_input.clone();
                window.defer(cx, move |window, cx| {
                    input.update(cx, |state, cx| state.focus(window, cx));
                });
            }
            NodeKind::Image { .. } => {
                return;
            }
        }

        self.selected_node = Some(id);
        cx.notify();
    }

    fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .selected_node
            .or_else(|| self.note.as_ref().map(|note| note.note_id));
        if let Some(id) = target {
            self.open_editor_for_node(id, window, cx);
        }
    }

    fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        self.editor_resize = None;
        let zh = self.is_zh();

        match editor.target {
            EditTarget::Note => {
                let value = self.body_input.read(cx).value().to_string();
                if let Some(note) = &mut self.note {
                    note.content = value;
                    note.sync_note_content();
                    let mut error = None;
                    if let Err(save_error) = note.save_content() {
                        error = Some(if zh {
                            format!("保存笔记失败: {save_error:#}")
                        } else {
                            format!("Failed to save note: {save_error:#}")
                        });
                    }
                    if let Err(save_error) = note.save_board() {
                        error = Some(if zh {
                            format!("保存白板失败: {save_error:#}")
                        } else {
                            format!("Failed to save board: {save_error:#}")
                        });
                    }
                    self.status = error.unwrap_or_else(|| {
                        if zh {
                            "笔记已保存".to_string()
                        } else {
                            "Note saved".to_string()
                        }
                    });
                }
            }
            EditTarget::Text(id) => {
                let value = self.body_input.read(cx).value().to_string();
                let format = editor.format;
                if let Some(note) = &mut self.note {
                    if let Some(node) = note.board.node_mut(id) {
                        node.kind = NodeKind::Text {
                            content: value,
                            format,
                        };
                    }
                    self.status = match note.save_board() {
                        Ok(()) if zh => "文本卡片已保存".to_string(),
                        Ok(()) => "Text card saved".to_string(),
                        Err(error) if zh => format!("保存失败: {error:#}"),
                        Err(error) => format!("Save failed: {error:#}"),
                    };
                }
            }
            EditTarget::Mind(id) => {
                let mut value = self.title_input.read(cx).value().to_string();
                if value.trim().is_empty() {
                    value = if zh {
                        "未命名节点".to_string()
                    } else {
                        "Untitled node".to_string()
                    };
                }
                let font_family = cx.theme().font_family.clone();
                if let Some(note) = &mut self.note {
                    if let Some(node) = note.board.node_mut(id) {
                        if let NodeKind::Mind { title, .. } = &mut node.kind {
                            *title = value;
                        }
                        fit_mind_node_height(node, window, &font_family);
                    }
                    resolve_mind_overlaps(&mut note.board.nodes);
                    self.status = match note.save_board() {
                        Ok(()) if zh => "思维导图节点已保存".to_string(),
                        Ok(()) => "Mind map node saved".to_string(),
                        Err(error) if zh => format!("保存失败: {error:#}"),
                        Err(error) => format!("Save failed: {error:#}"),
                    };
                }
            }
        }

        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn cancel_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.editor_resize = None;
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn viewport_center_world(&self) -> (f32, f32) {
        let Some(bounds) = self.board_bounds else {
            return (400.0, 300.0);
        };
        let Some(note) = &self.note else {
            return (400.0, 300.0);
        };
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        note.board.camera.screen_to_world(width / 2.0, height / 2.0)
    }

    fn cascade_offset(&self) -> (f32, f32) {
        let index = self
            .note
            .as_ref()
            .map(|note| note.board.nodes.len() % 6)
            .unwrap_or(0) as f32;
        (index * 22.0, index * 18.0)
    }

    fn add_text_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (center_x, center_y) = self.viewport_center_world();
        let (offset_x, offset_y) = self.cascade_offset();
        let zh = self.is_zh();
        let Some(note) = &mut self.note else {
            return;
        };
        let node = BoardNode::text(
            center_x - 170.0 + offset_x,
            center_y - 105.0 + offset_y,
            if zh {
                "## 新文本\n\n双击编辑内容，支持 Markdown。"
            } else {
                "## New text\n\nDouble-click to edit. Markdown is supported."
            },
            TextFormat::Markdown,
        );
        let id = node.id;
        note.board.nodes.push(node);
        self.multi_selected.clear();
        self.group_drag = None;
        self.selected_node = Some(id);
        self.status = match note.save_board() {
            Ok(()) if zh => "已添加文本卡片".to_string(),
            Ok(()) => "Text card added".to_string(),
            Err(error) if zh => format!("保存失败: {error:#}"),
            Err(error) => format!("Save failed: {error:#}"),
        };
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn add_mind_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (center_x, center_y) = self.viewport_center_world();
        let (offset_x, offset_y) = self.cascade_offset();
        let zh = self.is_zh();
        let Some(note) = &mut self.note else {
            return;
        };
        let font_family = cx.theme().font_family.clone();
        let mut node = BoardNode::mind(
            0.0,
            0.0,
            if zh {
                "��������"
            } else {
                "Central topic"
            },
            None,
        );
        fit_mind_node_height(&mut node, window, &font_family);
        let (node_x, node_y) = find_free_node_position(
            &note.board.nodes,
            center_x - node.w / 2.0 + offset_x,
            center_y - node.h / 2.0 + offset_y,
            node.w,
            node.h,
        );
        node.x = node_x;
        node.y = node_y;
        let id = node.id;
        note.board.nodes.push(node);
        self.multi_selected.clear();
        self.group_drag = None;
        self.selected_node = Some(id);
        self.status = if zh {
            "已添加思维导图节点，双击编辑标题"
        } else {
            "Mind map node added. Double-click to edit its title."
        }
        .to_string();
        let _ = note.save_board();
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn add_mind_child(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.selected_node else {
            return;
        };
        let zh = self.is_zh();
        let Some(note) = &mut self.note else {
            return;
        };
        let Some(parent) = note.board.node(selected) else {
            return;
        };
        if !parent.kind.is_mind() {
            self.status = if zh {
                "请先选中一个思维导图节点"
            } else {
                "Select a mind map node first"
            }
            .to_string();
            cx.notify();
            return;
        }

        let (parent_x, parent_y, parent_w) = (parent.x, parent.y, parent.w);
        let child_count = note
            .board
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    &node.kind,
                    NodeKind::Mind {
                        parent: Some(parent_id),
                        ..
                    } if *parent_id == selected
                )
            })
            .count() as f32;
        let offset_y = (child_count - 0.5) * 82.0;
        let font_family = cx.theme().font_family.clone();
        let mut child = BoardNode::mind(
            parent_x + parent_w + 78.0,
            parent_y + offset_y,
            if zh { "������" } else { "Subtopic" },
            Some(selected),
        );
        fit_mind_node_height(&mut child, window, &font_family);
        let child_id = child.id;
        note.board.nodes.push(child);
        resolve_mind_overlaps(&mut note.board.nodes);
        self.multi_selected.clear();
        self.group_drag = None;
        self.selected_node = Some(child_id);
        self.status = if zh {
            "已添加子节点，双击编辑标题"
        } else {
            "Child node added. Double-click to edit its title."
        }
        .to_string();
        let _ = note.save_board();
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_node else {
            return;
        };
        let zh = self.is_zh();
        let Some(note) = &mut self.note else {
            return;
        };
        let Some(index) = note.board.nodes.iter().position(|node| node.id == id) else {
            return;
        };
        if note.board.nodes[index].kind.is_note() {
            self.status = if zh {
                "笔记主卡片不能删除"
            } else {
                "The main note card cannot be deleted"
            }
            .to_string();
            cx.notify();
            return;
        }

        let removed = note.board.nodes.remove(index);
        if removed.kind.is_mind() {
            for node in &mut note.board.nodes {
                if let NodeKind::Mind { parent, .. } = &mut node.kind
                    && *parent == Some(id)
                {
                    *parent = None;
                }
            }
        }
        self.multi_selected.clear();
        self.group_drag = None;
        self.selected_node = Some(note.note_id);
        self.status = if zh {
            "已删除节点".to_string()
        } else {
            "Node deleted".to_string()
        };
        let _ = note.save_board();
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn toggle_node_format(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let zh = self.is_zh();
        let result = if let Some(note) = &mut self.note {
            let Some(node) = note.board.node_mut(id) else {
                return;
            };
            let NodeKind::Text { content: _, format } = &mut node.kind else {
                return;
            };
            *format = match *format {
                TextFormat::Markdown => TextFormat::Plain,
                TextFormat::Plain => TextFormat::Markdown,
            };
            let next = *format;
            let result = note.save_board();
            Some((next, result))
        } else {
            None
        };

        if let Some((next, result)) = result {
            let label = match (zh, next) {
                (true, TextFormat::Markdown) => "Markdown",
                (true, TextFormat::Plain) => "纯文本",
                (false, TextFormat::Markdown) => "Markdown",
                (false, TextFormat::Plain) => "plain text",
            };
            self.status = match result {
                Ok(()) if zh => format!("已切换为{label}"),
                Ok(()) => format!("Switched to {label}"),
                Err(error) if zh => format!("保存失败: {error:#}"),
                Err(error) => format!("Save failed: {error:#}"),
            };
            cx.notify();
        }
    }

    fn set_node_rotation(&mut self, id: Uuid, rotation: f32, cx: &mut Context<Self>) {
        let zh = self.is_zh();
        let result = if let Some(note) = &mut self.note {
            let Some(node) = note.board.node_mut(id) else {
                return;
            };
            node.rotation = rotation.rem_euclid(360.0);
            let rotation = node.rotation;
            let result = note.save_board();
            Some((rotation, result))
        } else {
            None
        };

        if let Some((rotation, result)) = result {
            self.rotated_svg_cache.borrow_mut().clear();
            self.status = match result {
                Ok(()) if zh => format!("已旋转到 {:.0}°", rotation),
                Ok(()) => format!("Rotated to {:.0}°", rotation),
                Err(error) if zh => format!("保存失败: {error:#}"),
                Err(error) => format!("Save failed: {error:#}"),
            };
            cx.notify();
        }
    }

    fn rotate_node(&mut self, id: Uuid, delta: f32, cx: &mut Context<Self>) {
        let Some(current) = self
            .note
            .as_ref()
            .and_then(|note| note.board.node(id))
            .map(|node| node.rotation)
        else {
            return;
        };
        self.set_node_rotation(id, current + delta, cx);
    }

    fn cached_rotated_svg(
        &self,
        key: u64,
        build: impl FnOnce() -> Option<String>,
    ) -> Option<Arc<Image>> {
        let mut cache = self.rotated_svg_cache.borrow_mut();
        if let Some(image) = cache.get(&key) {
            return Some(image.clone());
        }
        let svg = build()?;
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()));
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(key, image.clone());
        Some(image)
    }

    fn start_resize(
        &mut self,
        id: Uuid,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.board_bounds else {
            return;
        };
        let local_x = f32::from(event.position.x - bounds.origin.x);
        let local_y = f32::from(event.position.y - bounds.origin.y);
        let Some((start_w, start_h)) = self
            .note
            .as_ref()
            .and_then(|note| note.board.node(id))
            .map(|node| (node.w, node.h))
        else {
            return;
        };

        self.select_single_node(id);
        self.interaction = Interaction::Resizing {
            id,
            start_x: local_x,
            start_y: local_y,
            start_w,
            start_h,
        };
        self.board_focus.focus(window, cx);
        cx.stop_propagation();
        cx.notify();
    }

    fn paste_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let zh = self.is_zh();
        if self.note.is_none() {
            self.status = if zh {
                "请先打开一篇笔记"
            } else {
                "Open a note first"
            }
            .to_string();
            cx.notify();
            return;
        }

        match read_clipboard_image(&self.root) {
            Ok(Some((path, width, height))) => {
                let (display_width, display_height) = fit_image(width, height);
                let (center_x, center_y) = self.viewport_center_world();
                let (offset_x, offset_y) = self.cascade_offset();
                let relative = path
                    .strip_prefix(&self.root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");

                let mut node = BoardNode::image(
                    center_x - display_width / 2.0 + offset_x,
                    center_y - display_height / 2.0 + offset_y,
                    relative,
                    display_width,
                    display_height,
                );
                node.w = display_width;
                node.h = display_height;
                let id = node.id;

                if let Some(note) = &mut self.note {
                    note.board.nodes.push(node);
                    self.multi_selected.clear();
                    self.group_drag = None;
                    self.selected_node = Some(id);
                    self.status = if zh {
                        "已粘贴剪贴板图片".to_string()
                    } else {
                        "Pasted image from clipboard".to_string()
                    };
                    let _ = note.save_board();
                }
            }
            Ok(None) => {
                if let Some(text) = read_clipboard_text() {
                    let (center_x, center_y) = self.viewport_center_world();
                    let (offset_x, offset_y) = self.cascade_offset();
                    if let Some(note) = &mut self.note {
                        let node = BoardNode::text(
                            center_x - 170.0 + offset_x,
                            center_y - 105.0 + offset_y,
                            text,
                            TextFormat::Plain,
                        );
                        let id = node.id;
                        note.board.nodes.push(node);
                        self.multi_selected.clear();
                        self.group_drag = None;
                        self.selected_node = Some(id);
                        self.status = if zh {
                            "剪贴板没有图片，已粘贴为纯文本卡片".to_string()
                        } else {
                            "No image in clipboard; pasted as a plain-text card".to_string()
                        };
                        let _ = note.save_board();
                    }
                } else {
                    self.status = if zh {
                        "剪贴板中没有图片或文本".to_string()
                    } else {
                        "Clipboard has no image or text".to_string()
                    };
                }
            }
            Err(error) => {
                self.status = if zh {
                    format!("粘贴图片失败: {error:#}")
                } else {
                    format!("Failed to paste image: {error:#}")
                };
            }
        }

        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn zoom_at_center(&mut self, factor: f32) {
        let Some(bounds) = self.board_bounds else {
            return;
        };
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let Some(note) = &mut self.note else {
            return;
        };
        let zoom = (note.board.camera.zoom * factor).clamp(0.2, 3.0);
        note.board
            .camera
            .zoom_around(width / 2.0, height / 2.0, zoom);
    }

    fn zoom_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_at_center(1.2);
        self.save_current();
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn zoom_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_at_center(1.0 / 1.2);
        self.save_current();
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn should_fit_board(&self, note: &NoteDoc) -> bool {
        let camera = note.board.camera;
        let is_default = (camera.zoom - 1.0).abs() < 0.001
            && (camera.pan_x - 48.0).abs() < 0.5
            && (camera.pan_y - 48.0).abs() < 0.5;
        if is_default {
            return true;
        }

        let Some(bounds) = self.board_bounds else {
            return false;
        };
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        !note.board.nodes.iter().all(|node| {
            let (x, y) = camera.world_to_screen(node.x, node.y);
            x >= -12.0
                && y >= -12.0
                && x + node.w * camera.zoom <= width + 12.0
                && y + node.h * camera.zoom <= height + 12.0
        })
    }

    fn fit_view_to_size(&mut self, width: f32, height: f32) {
        let Some(note) = &mut self.note else {
            return;
        };

        if note.board.nodes.is_empty() {
            note.board.camera = Camera::default();
            return;
        }

        let min_x = note
            .board
            .nodes
            .iter()
            .map(|node| node.x)
            .fold(f32::INFINITY, f32::min);
        let min_y = note
            .board
            .nodes
            .iter()
            .map(|node| node.y)
            .fold(f32::INFINITY, f32::min);
        let max_x = note
            .board
            .nodes
            .iter()
            .map(|node| node.x + node.w)
            .fold(f32::NEG_INFINITY, f32::max);
        let max_y = note
            .board
            .nodes
            .iter()
            .map(|node| node.y + node.h)
            .fold(f32::NEG_INFINITY, f32::max);

        let content_width = (max_x - min_x).max(1.0);
        let content_height = (max_y - min_y).max(1.0);
        let zoom = ((width - 100.0) / content_width)
            .min((height - 100.0) / content_height)
            .clamp(0.2, 1.0);

        let camera = &mut note.board.camera;
        camera.zoom = zoom;
        camera.pan_x = width / 2.0 - (min_x + content_width / 2.0) * zoom;
        camera.pan_y = height / 2.0 - (min_y + content_height / 2.0) * zoom;
    }

    fn reset_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.board_bounds else {
            return;
        };
        self.fit_view_to_size(f32::from(bounds.size.width), f32::from(bounds.size.height));
        self.status = self
            .tr("已适应画布内容", "Canvas fitted to content")
            .to_string();
        if let Some(note) = &self.note {
            let _ = note.save_board();
        }
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn board_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        self.selected_dir = None;
        self.selected_tree = None;
        self.board_focus.focus(window, cx);

        let Some(bounds) = self.board_bounds else {
            return;
        };
        let local_x = f32::from(event.position.x - bounds.origin.x);
        let local_y = f32::from(event.position.y - bounds.origin.y);

        let resize_handle_hit = self.note.as_ref().and_then(|note| {
            let id = self.selected_node?;
            let node = note.board.node(id)?;
            if !matches!(
                node.kind,
                NodeKind::Note { .. } | NodeKind::Text { .. } | NodeKind::Image { .. }
            ) {
                return None;
            }
            let (handle_x, handle_y) = note
                .board
                .camera
                .world_to_screen(node.x + node.w, node.y + node.h);
            ((local_x - handle_x).abs() <= 13.0 && (local_y - handle_y).abs() <= 13.0).then_some(id)
        });
        if let Some(id) = resize_handle_hit {
            self.start_resize(id, event, window, cx);
            return;
        }

        let hit = {
            let Some(note) = &self.note else {
                return;
            };
            let (world_x, world_y) = note.board.camera.screen_to_world(local_x, local_y);
            note.board
                .nodes
                .iter()
                .rev()
                .find(|node| node.contains(world_x, world_y))
                .map(|node| {
                    (
                        node.id,
                        world_x,
                        world_y,
                        node.x,
                        node.y,
                        node.kind.is_mind(),
                    )
                })
        };

        if let Some((id, world_x, world_y, node_x, node_y, is_mind)) = hit {
            if event.click_count >= 2 {
                self.select_single_node(id);
                self.open_editor_for_node(id, window, cx);
                return;
            }

            if event.modifiers.shift && is_mind {
                if self.multi_selected.contains(&id) {
                    self.multi_selected.remove(&id);
                    self.selected_node = self.multi_selected.iter().next().copied();
                } else {
                    self.multi_selected.insert(id);
                    self.selected_node = Some(id);
                }
                self.group_drag = None;
                self.interaction = Interaction::None;
                cx.notify();
                return;
            }

            let in_multi = self.multi_selected.contains(&id) && self.multi_selected.len() > 1;
            let positions = self
                .note
                .as_ref()
                .map(|note| {
                    let roots = if in_multi {
                        self.multi_selected.iter().copied().collect::<Vec<_>>()
                    } else {
                        vec![id]
                    };
                    let mut drag_ids = HashSet::new();
                    for root in roots {
                        if note
                            .board
                            .node(root)
                            .is_some_and(|node| node.kind.is_mind())
                        {
                            drag_ids.extend(mind_subtree_ids(&note.board.nodes, root));
                        } else {
                            drag_ids.insert(root);
                        }
                    }
                    note.board
                        .nodes
                        .iter()
                        .filter(|node| drag_ids.contains(&node.id))
                        .map(|node| (node.id, node.x, node.y))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let group_drag = if positions.len() > 1 {
                Some(GroupDrag {
                    anchor_x: node_x,
                    anchor_y: node_y,
                    nodes: positions,
                })
            } else {
                None
            };

            if !in_multi {
                self.multi_selected.clear();
            }
            self.selected_node = Some(id);
            self.group_drag = group_drag;
            self.interaction = Interaction::Dragging {
                id,
                offset_x: world_x - node_x,
                offset_y: world_y - node_y,
            };
        } else {
            self.group_drag = None;
            if event.modifiers.shift {
                self.interaction = Interaction::BoxSelecting {
                    start_x: local_x,
                    start_y: local_y,
                    current_x: local_x,
                    current_y: local_y,
                };
            } else {
                self.clear_node_selection();
                if let Some(note) = &self.note {
                    let camera = note.board.camera;
                    self.interaction = Interaction::Panning {
                        start_x: local_x,
                        start_y: local_y,
                        pan_x: camera.pan_x,
                        pan_y: camera.pan_y,
                    };
                }
            }
        }

        cx.notify();
    }

    fn board_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.board_bounds else {
            return;
        };
        let local_x = f32::from(event.position.x - bounds.origin.x);
        let local_y = f32::from(event.position.y - bounds.origin.y);

        if event.pressed_button != Some(MouseButton::Left) {
            if !matches!(self.interaction, Interaction::None) {
                self.interaction = Interaction::None;
                if let Some(note) = &self.note {
                    let _ = note.save_board();
                }
                cx.notify();
            }
            return;
        }

        match self.interaction {
            Interaction::Panning {
                start_x,
                start_y,
                pan_x,
                pan_y,
            } => {
                if let Some(note) = &mut self.note {
                    note.board.camera.pan_x = pan_x + local_x - start_x;
                    note.board.camera.pan_y = pan_y + local_y - start_y;
                    cx.notify();
                }
            }
            Interaction::Dragging {
                id,
                offset_x,
                offset_y,
            } => {
                let group = self.group_drag.clone();
                let Some(note) = &mut self.note else {
                    return;
                };
                let camera = note.board.camera;
                let (world_x, world_y) = camera.screen_to_world(local_x, local_y);

                if let Some(group) = group {
                    let delta_x = (world_x - offset_x) - group.anchor_x;
                    let delta_y = (world_y - offset_y) - group.anchor_y;
                    for (node_id, x, y) in group.nodes {
                        if let Some(node) = note.board.node_mut(node_id) {
                            node.x = (x + delta_x).round();
                            node.y = (y + delta_y).round();
                        }
                    }
                } else if let Some(node) = note.board.node_mut(id) {
                    node.x = (world_x - offset_x).round();
                    node.y = (world_y - offset_y).round();
                }
                cx.notify();
            }
            Interaction::BoxSelecting {
                start_x, start_y, ..
            } => {
                self.interaction = Interaction::BoxSelecting {
                    start_x,
                    start_y,
                    current_x: local_x,
                    current_y: local_y,
                };
                cx.notify();
            }
            Interaction::Resizing {
                id,
                start_x,
                start_y,
                start_w,
                start_h,
            } => {
                let Some(note) = &mut self.note else {
                    return;
                };
                let zoom = note.board.camera.zoom.max(0.2);
                let delta_w = (local_x - start_x) / zoom;
                let delta_h = (local_y - start_y) / zoom;
                if let Some(node) = note.board.node_mut(id) {
                    node.w = (start_w + delta_w).clamp(80.0, 2400.0);
                    node.h = (start_h + delta_h).clamp(50.0, 2000.0);
                    cx.notify();
                }
            }
            Interaction::None => {}
        }
    }

    fn board_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Interaction::BoxSelecting {
            start_x,
            start_y,
            current_x,
            current_y,
        } = self.interaction
        {
            let (selection, primary) = if let Some(note) = &self.note {
                let camera = note.board.camera;
                let (world_x1, world_y1) =
                    camera.screen_to_world(start_x.min(current_x), start_y.min(current_y));
                let (world_x2, world_y2) =
                    camera.screen_to_world(start_x.max(current_x), start_y.max(current_y));
                let selected = note
                    .board
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.kind.is_mind()
                            && node.x < world_x2
                            && node.x + node.w > world_x1
                            && node.y < world_y2
                            && node.y + node.h > world_y1
                    })
                    .map(|node| node.id)
                    .collect::<HashSet<_>>();
                let primary = note
                    .board
                    .nodes
                    .iter()
                    .rev()
                    .find(|node| selected.contains(&node.id))
                    .map(|node| node.id);
                (selected, primary)
            } else {
                (HashSet::new(), None)
            };

            if selection.is_empty() {
                self.clear_node_selection();
            } else {
                self.multi_selected = selection;
                self.selected_node = primary;
                self.group_drag = None;
            }
            self.interaction = Interaction::None;
            cx.notify();
            return;
        }

        if !matches!(self.interaction, Interaction::None) {
            let was_resizing = matches!(self.interaction, Interaction::Resizing { .. });
            if was_resizing {
                self.rotated_svg_cache.borrow_mut().clear();
            }
            self.interaction = Interaction::None;
            self.group_drag = None;
            if let Some(note) = &mut self.note {
                if was_resizing {
                    let font_family = cx.theme().font_family.clone();
                    if fit_mind_node_heights(&mut note.board.nodes, window, &font_family) {
                        resolve_mind_overlaps(&mut note.board.nodes);
                    }
                }
                let _ = note.save_board();
            }
            cx.notify();
        }
    }

    fn board_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.board_bounds else {
            return;
        };
        let local_x = f32::from(event.position.x - bounds.origin.x);
        let local_y = f32::from(event.position.y - bounds.origin.y);

        let (delta_x, delta_y) = match event.delta {
            ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            ScrollDelta::Lines(delta) => (delta.x * 24.0, delta.y * 24.0),
        };

        let Some(note) = &mut self.note else {
            return;
        };

        if event.modifiers.control || event.modifiers.platform {
            let factor = (delta_y * 0.0025).exp();
            let zoom = (note.board.camera.zoom * factor).clamp(0.2, 3.0);
            note.board.camera.zoom_around(local_x, local_y, zoom);
        } else {
            note.board.camera.pan_x -= delta_x;
            note.board.camera.pan_y -= delta_y;
        }

        cx.notify();
    }

    fn board_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;

        if modifiers.control {
            match key.to_ascii_lowercase().as_str() {
                "v" => self.paste_image(window, cx),
                "n" if modifiers.shift => self.open_create_dialog(CreateMode::Folder, window, cx),
                "n" => self.open_create_dialog(CreateMode::Markdown, window, cx),
                "e" => self.edit_selected(window, cx),
                _ => {}
            }
            return;
        }

        if !modifiers.alt && !modifiers.platform {
            match key {
                "delete" | "backspace" => self.delete_selected(window, cx),
                "=" | "+" => self.zoom_in(window, cx),
                "-" => self.zoom_out(window, cx),
                "0" => self.reset_view(window, cx),
                "escape" => {
                    self.clear_node_selection();
                    cx.notify();
                }
                _ => {}
            }
        }
    }

    fn editor_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            self.cancel_editor(window, cx);
        } else if (event.keystroke.modifiers.control
            && event.keystroke.key.eq_ignore_ascii_case("enter"))
            || (event.keystroke.key == "enter"
                && matches!(
                    self.editor.map(|editor| editor.target),
                    Some(EditTarget::Mind(_))
                ))
        {
            self.save_editor(window, cx);
        }
    }

    fn start_editor_resize(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor_resize = Some(EditorResize {
            start_x: f32::from(event.position.x),
            start_y: f32::from(event.position.y),
            start_width: self.editor_width,
            start_height: self.editor_height,
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn editor_resize_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(resize) = self.editor_resize else {
            return;
        };
        let viewport = window.viewport_size();
        let max_width = (f32::from(viewport.width) - 40.0).max(480.0);
        let max_height = (f32::from(viewport.height) - 40.0).max(320.0);
        let width = (resize.start_width + (f32::from(event.position.x) - resize.start_x) * 2.0)
            .clamp(480.0, max_width);
        let height = (resize.start_height + (f32::from(event.position.y) - resize.start_y) * 2.0)
            .clamp(320.0, max_height);
        if (self.editor_width - width).abs() > 0.01 || (self.editor_height - height).abs() > 0.01 {
            self.editor_width = width;
            self.editor_height = height;
            cx.notify();
        }
    }

    fn finish_editor_resize(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor_resize.take().is_some() {
            cx.notify();
        }
    }

    fn render_custom_titlebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let background = cx.theme().background;
        let border = cx.theme().border;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let primary = cx.theme().primary;
        let hover_bg = cx.theme().muted;
        let danger = cx.theme().danger;
        let danger_fg = cx.theme().danger_foreground;
        let maximize_icon = if window.is_maximized() {
            IconName::WindowRestore
        } else {
            IconName::WindowMaximize
        };

        h_flex()
            .h(px(34.))
            .w_full()
            .flex_shrink_0()
            .bg(background)
            .border_b_1()
            .border_color(border)
            .child(
                h_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .pl_3()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        Icon::new(IconName::NotebookPen)
                            .size(px(15.))
                            .text_color(primary),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_bold()
                            .text_color(foreground)
                            .child(self.tr("白板笔记", "Whiteboard Notes")),
                    ),
            )
            .child(window_control_button(
                "titlebar-minimize",
                WindowControlArea::Min,
                IconName::WindowMinimize,
                muted,
                hover_bg,
                foreground,
            ))
            .child(window_control_button(
                "titlebar-maximize",
                WindowControlArea::Max,
                maximize_icon,
                muted,
                hover_bg,
                foreground,
            ))
            .child(window_control_button(
                "titlebar-close",
                WindowControlArea::Close,
                IconName::WindowClose,
                muted,
                danger,
                danger_fg,
            ))
            .into_any_element()
    }

    fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let sidebar = cx.theme().sidebar;
        let sidebar_fg = cx.theme().sidebar_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().sidebar_accent;
        let accent_fg = cx.theme().sidebar_accent_foreground;
        let primary = cx.theme().primary;
        let primary_fg = cx.theme().primary_foreground;

        let entries = self.entries.clone();
        let selected_note = self.selected_note.clone();
        let selected_dir = self.selected_dir.clone();
        let selected_tree = self.selected_tree.clone();
        let expanded = self.expanded.clone();
        let root = self.root.clone();
        let note_count = entries.iter().filter(|entry| !entry.is_dir).count();
        let folder_count = entries.iter().filter(|entry| entry.is_dir).count();

        v_flex()
            .h_full()
            .w_full()
            .bg(sidebar)
            .text_color(sidebar_fg)
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(34.))
                            .rounded(px(10.))
                            .bg(primary)
                            .text_color(primary_fg)
                            .child(Icon::new(IconName::NotebookPen).size(px(18.))),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_bold()
                                    .child(self.tr("白板笔记", "Whiteboard Notes")),
                            )
                            .child(div().text_xs().text_color(muted).child(
                                self.tr("无限画布 · Markdown", "Infinite canvas · Markdown"),
                            )),
                    )
                    .child(
                        Button::new("collapse-sidebar")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelLeftClose)
                            .tooltip(self.tr("收起侧边栏", "Collapse sidebar"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.collapse_sidebar(cx);
                            })),
                    )
                    .child(
                        Button::new("open-settings")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings)
                            .tooltip(self.tr("设置", "Settings"))
                            .selected(self.settings_open)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_open = !this.settings_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .pb_2()
                    .child(
                        Button::new("new-folder")
                            .ghost()
                            .small()
                            .icon(IconName::FolderPlus)
                            .label(self.tr("目录", "Folder"))
                            .tooltip(self.tr("新建目录", "New folder"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_dialog(CreateMode::Folder, window, cx);
                            })),
                    )
                    .child(
                        Button::new("new-note")
                            .primary()
                            .small()
                            .icon(IconName::FilePlus)
                            .label(self.tr("笔记", "Note"))
                            .tooltip(self.tr(
                                "新建 Markdown 或纯文本笔记",
                                "New Markdown or plain-text note",
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_dialog(CreateMode::Markdown, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("note-tree")
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .pb_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.selected_dir.is_some() || this.selected_tree.is_some() {
                            this.selected_dir = None;
                            this.selected_tree = None;
                            cx.notify();
                        }
                    }))
                    .overflow_y_scrollbar()
                    .children(entries.into_iter().map(|entry| {
                        let path = entry.path.clone();
                        let click_path = path.clone();
                        let is_dir = entry.is_dir;
                        let is_selected = if is_dir {
                            selected_dir.as_deref() == Some(path.as_path())
                        } else {
                            selected_note.as_deref() == Some(path.as_path())
                        };
                        let is_expanded = is_dir && expanded.contains(&path);
                        let row_id = format!("tree-row-{}", path.display());
                        let hover_bg = accent.opacity(0.55);
                        let row_bg = if is_selected { accent } else { sidebar };
                        let row_fg = if is_selected { accent_fg } else { sidebar_fg };
                        let depth = entry.depth as f32;
                        let format = entry.format;
                        let name = entry.name.clone();
                        let action_path = path.clone();
                        let show_actions = selected_tree.as_deref() == Some(path.as_path());

                        div()
                            .id(SharedString::from(row_id))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(30.))
                            .pl(px(8.0 + depth * 14.0))
                            .pr_2()
                            .rounded(px(7.))
                            .bg(row_bg)
                            .text_color(row_fg)
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover_bg))
                            .when(is_dir, |this| {
                                this.child(
                                    Icon::new(if is_expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(13.))
                                    .text_color(muted),
                                )
                            })
                            .child(
                                Icon::new(if is_dir {
                                    if is_expanded {
                                        IconName::FolderOpen
                                    } else {
                                        IconName::Folder
                                    }
                                } else {
                                    IconName::FileText
                                })
                                .size(px(15.)),
                            )
                            .child(div().flex_1().min_w_0().overflow_hidden().text_sm().child(
                                if is_dir {
                                    name
                                } else {
                                    note_title(Path::new(&name))
                                },
                            ))
                            .when(!show_actions, |this| {
                                this.when_some(format, |this, format| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(format.short_label()),
                                    )
                                })
                            })
                            .when(show_actions, |this| {
                                this.child(
                                    Button::new(SharedString::from(format!(
                                        "rename-{}",
                                        action_path.display()
                                    )))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Pencil)
                                    .tooltip(self.tr("重命名", "Rename"))
                                    .on_click(cx.listener({
                                        let path = action_path.clone();
                                        move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.open_rename_dialog(path.clone(), window, cx);
                                        }
                                    })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "delete-{}",
                                        action_path.display()
                                    )))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Trash)
                                    .tooltip(self.tr("删除", "Delete"))
                                    .on_click(cx.listener({
                                        let path = action_path.clone();
                                        move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.open_delete_dialog(path.clone(), cx);
                                        }
                                    })),
                                )
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.selected_tree = Some(click_path.clone());
                                if is_dir {
                                    this.selected_dir = Some(click_path.clone());
                                    if this.expanded.contains(&click_path) {
                                        this.expanded.remove(&click_path);
                                    } else {
                                        this.expanded.insert(click_path.clone());
                                    }
                                    this.refresh_tree();
                                    cx.notify();
                                } else {
                                    this.selected_dir = None;
                                    this.settings_open = false;
                                    this.open_note(&click_path, window, cx);
                                }
                            }))
                            .into_any_element()
                    })),
            )
            .child(
                div()
                    .id("sidebar-footer")
                    .border_t_1()
                    .border_color(border)
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.selected_dir.is_some() || this.selected_tree.is_some() {
                            this.selected_dir = None;
                            this.selected_tree = None;
                            cx.notify();
                        }
                    }))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(if self.is_zh() {
                                        format!("{note_count} 篇笔记")
                                    } else if note_count == 1 {
                                        "1 note".to_string()
                                    } else {
                                        format!("{note_count} notes")
                                    })
                                    .child(if self.is_zh() {
                                        format!("{folder_count} 个目录")
                                    } else if folder_count == 1 {
                                        "1 folder".to_string()
                                    } else {
                                        format!("{folder_count} folders")
                                    }),
                            )
                            .child(div().text_xs().text_color(muted).overflow_hidden().child(
                                if self.is_zh() {
                                    format!(
                                        "保存位置：{}",
                                        root.file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or("WhiteboardNotes")
                                    )
                                } else {
                                    format!(
                                        "Saved in: {}",
                                        root.file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or("WhiteboardNotes")
                                    )
                                },
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_toolbar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let note_title = self
            .note
            .as_ref()
            .map(|note| note.title.clone())
            .unwrap_or_else(|| self.tr("未打开笔记", "No note open").to_string());
        let node_count = self
            .note
            .as_ref()
            .map(|note| note.board.nodes.len())
            .unwrap_or(0);
        let zoom = self
            .note
            .as_ref()
            .map(|note| note.board.camera.zoom)
            .unwrap_or(1.0);
        let selected_is_mind = self
            .selected_node
            .and_then(|id| self.note.as_ref().and_then(|note| note.board.node(id)))
            .map(|node| node.kind.is_mind())
            .unwrap_or(false);
        let selected_is_note = self
            .selected_node
            .and_then(|id| self.note.as_ref().and_then(|note| note.board.node(id)))
            .map(|node| node.kind.is_note())
            .unwrap_or(false);
        let has_selection = self.selected_node.is_some();

        let bg = cx.theme().background;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;

        h_flex()
            .h(px(56.))
            .px_3()
            .gap_2()
            .bg(bg)
            .border_b_1()
            .border_color(border)
            .child(
                Button::new("toggle-sidebar")
                    .ghost()
                    .small()
                    .icon(if self.sidebar_collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .tooltip(if self.sidebar_collapsed {
                        self.tr("展开侧边栏", "Expand sidebar")
                    } else {
                        self.tr("收起侧边栏", "Collapse sidebar")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_sidebar(cx);
                    })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_bold()
                            .overflow_hidden()
                            .child(note_title),
                    )
                    .child(div().text_xs().text_color(muted).child(if self.is_zh() {
                        format!("{node_count} 个节点 · 滚轮平移 · Ctrl+滚轮缩放")
                    } else {
                        let node_word = if node_count == 1 { "node" } else { "nodes" };
                        format!("{node_count} {node_word} · wheel to pan · Ctrl+wheel to zoom")
                    })),
            )
            .child(
                Button::new("add-text")
                    .ghost()
                    .small()
                    .icon(IconName::StickyNote)
                    .label(self.tr("文本", "Text"))
                    .tooltip(self.tr("添加文本卡片", "Add a text card"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_text_node(window, cx);
                    })),
            )
            .child(
                Button::new("add-mind")
                    .ghost()
                    .small()
                    .icon(IconName::Brain)
                    .label(self.tr("思维导图", "Mind map"))
                    .tooltip(self.tr("添加思维导图节点", "Add a mind map node"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_mind_root(window, cx);
                    })),
            )
            .child(
                Button::new("paste-image")
                    .ghost()
                    .small()
                    .icon(IconName::Image)
                    .label(self.tr("粘贴图片", "Paste image"))
                    .tooltip(self.tr(
                        "粘贴剪贴板图片（Ctrl+V）",
                        "Paste an image from the clipboard (Ctrl+V)",
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.paste_image(window, cx);
                    })),
            )
            .child(div().w(px(1.)).h(px(22.)).bg(border))
            .child(
                Button::new("edit-card")
                    .ghost()
                    .small()
                    .icon(IconName::Pencil)
                    .label(if selected_is_note {
                        self.tr("编辑笔记", "Edit note")
                    } else {
                        self.tr("编辑", "Edit")
                    })
                    .tooltip(self.tr("编辑选中的卡片", "Edit the selected card"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.edit_selected(window, cx);
                    })),
            )
            .when(selected_is_mind && !selected_is_note, |this| {
                this.child(
                    Button::new("add-child")
                        .ghost()
                        .small()
                        .icon(IconName::GitBranch)
                        .label(self.tr("子节点", "Child"))
                        .tooltip(self.tr(
                            "为选中的思维导图节点添加子节点",
                            "Add a child to the selected mind map node",
                        ))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_mind_child(window, cx);
                        })),
                )
            })
            .when(has_selection && !selected_is_note, |this| {
                this.child(
                    Button::new("delete-node")
                        .ghost()
                        .small()
                        .icon(IconName::Trash)
                        .label(self.tr("删除", "Delete"))
                        .tooltip(self.tr("删除选中的节点", "Delete the selected node"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.delete_selected(window, cx);
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("zoom-out")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Minus)
                    .tooltip(self.tr("缩小", "Zoom out"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_out(window, cx);
                    })),
            )
            .child(
                Button::new("zoom-reset")
                    .ghost()
                    .xsmall()
                    .label(format!("{}%", (zoom * 100.0).round()))
                    .tooltip(self.tr("适应画布内容", "Fit canvas to content"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.reset_view(window, cx);
                    })),
            )
            .child(
                Button::new("zoom-in")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip(self.tr("放大", "Zoom in"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_in(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_canvas(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(note) = &self.note else {
            return div().into_any_element();
        };

        let camera = note.board.camera;
        let selected = self.selected_node;
        let mut edges = Vec::new();
        for node in &note.board.nodes {
            if let NodeKind::Mind {
                parent: Some(parent_id),
                ..
            } = &node.kind
                && let Some(parent) = note.board.node(*parent_id)
            {
                edges.push((
                    parent.x + parent.w,
                    parent.y + parent.h / 2.0,
                    node.x,
                    node.y + node.h / 2.0,
                ));
            }
        }

        let border = cx.theme().border;
        let primary = cx.theme().primary;
        let edge_color = cx.theme().primary.opacity(0.55);

        let mut elements = Vec::with_capacity(note.board.nodes.len());
        for node in &note.board.nodes {
            let (screen_x, screen_y) = camera.world_to_screen(node.x, node.y);
            elements.push(
                self.render_node(node, screen_x, screen_y, camera.zoom, selected, cx)
                    .into_any_element(),
            );
        }

        let selection_box = match self.interaction {
            Interaction::BoxSelecting {
                start_x,
                start_y,
                current_x,
                current_y,
            } => Some(
                div()
                    .absolute()
                    .left(px(start_x.min(current_x)))
                    .top(px(start_y.min(current_y)))
                    .w(px((current_x - start_x).abs()))
                    .h(px((current_y - start_y).abs()))
                    .border_2()
                    .border_color(primary)
                    .bg(primary.opacity(0.08))
                    .into_any_element(),
            ),
            _ => None,
        };

        let resize_handle = self.note.as_ref().and_then(|note| {
            let id = self.selected_node?;
            let node = note.board.node(id)?;
            if !matches!(
                node.kind,
                NodeKind::Note { .. } | NodeKind::Text { .. } | NodeKind::Image { .. }
            ) {
                return None;
            }
            let (screen_x, screen_y) = camera.world_to_screen(node.x, node.y);
            let handle_size = 14.0;
            Some(
                div()
                    .id(SharedString::from(format!("resize-{id}")))
                    .absolute()
                    .left(px(screen_x + node.w * camera.zoom - handle_size / 2.0))
                    .top(px(screen_y + node.h * camera.zoom - handle_size / 2.0))
                    .size(px(handle_size))
                    .rounded(px(3.))
                    .bg(primary)
                    .border_2()
                    .border_color(cx.theme().background)
                    .cursor_nwse_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, window, cx| {
                            this.start_resize(id, event, window, cx);
                        }),
                    )
                    .into_any_element(),
            )
        });

        let view = cx.entity();
        let canvas_element = canvas(
            |bounds, _, _| bounds,
            move |_, bounds, window, _cx| {
                paint_grid(bounds, camera, border, window);
                paint_edges(bounds, camera, &edges, edge_color, window);
            },
        )
        .absolute()
        .size_full();

        div()
            .id("whiteboard")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .track_focus(&self.board_focus)
            .on_prepaint(move |bounds, _, cx| {
                view.update(cx, |this, _| {
                    this.board_bounds = Some(bounds);
                });
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::board_mouse_down))
            .on_mouse_move(cx.listener(Self::board_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::board_mouse_up))
            .on_scroll_wheel(cx.listener(Self::board_scroll))
            .on_key_down(cx.listener(Self::board_key_down))
            .child(canvas_element)
            .children(elements)
            .children(selection_box)
            .children(resize_handle)
            .into_any_element()
    }

    fn render_node(
        &self,
        node: &BoardNode,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        selected: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let node_id = node.id;
        let menu_view = cx.entity();
        let node_element_id = SharedString::from(format!("node-{node_id}"));
        let is_selected = selected == Some(node.id) || self.multi_selected.contains(&node.id);
        let border_color = if is_selected {
            theme.primary
        } else {
            theme.border
        };
        let width = (node.w * zoom).max(24.0);
        let height = (node.h * zoom).max(18.0);
        let padding = (10.0 * zoom).clamp(3.0, 14.0);
        let font_size = (13.0 * zoom).clamp(5.0, 26.0);
        let card_bg = theme.popover;
        let card_fg = theme.popover_foreground;
        let rotated = node.rotation.abs() > 0.01;
        let base_bg = if rotated { theme.transparent } else { card_bg };
        let base_border = if is_selected {
            theme.primary
        } else if rotated {
            theme.transparent
        } else {
            border_color
        };

        let base = div()
            .id(node_element_id.clone())
            .absolute()
            .left(px(screen_x))
            .top(px(screen_y))
            .w(px(width))
            .h(px(height))
            .rounded(px((10.0 * zoom).clamp(4.0, 14.0)))
            .border_2()
            .border_color(base_border)
            .bg(base_bg)
            .text_color(card_fg)
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    this.select_node_from_pointer(node_id);
                    cx.notify();
                }),
            )
            .context_menu(node_context_menu(
                menu_view.clone(),
                node_id,
                NodeMenuKind::Image,
                self.settings.language,
            ));

        match &node.kind {
            NodeKind::Note { content, format } | NodeKind::Text { content, format } => {
                let is_note = node.kind.is_note();
                let title = if is_note {
                    self.note
                        .as_ref()
                        .map(|note| note.title.clone())
                        .unwrap_or_else(|| self.tr("笔记", "Note").to_string())
                } else {
                    self.tr("文本卡片", "Text card").to_string()
                };
                let header_height = (32.0 * zoom).clamp(18.0, 34.0);
                let content = content.clone();
                let format = *format;
                let menu_kind = if is_note {
                    NodeMenuKind::Note
                } else {
                    NodeMenuKind::Text(format)
                };

                if rotated {
                    let key = rotated_svg_key(node, &title, Some(&content), Some(format), &theme);
                    let image = self.cached_rotated_svg(key, || {
                        Some(build_text_svg(
                            node.w,
                            node.h,
                            &title,
                            &content,
                            format,
                            node.rotation,
                            &theme,
                            is_note,
                        ))
                    });

                    if let Some(image) = image {
                        return v_flex()
                            .id(node_element_id.clone())
                            .absolute()
                            .left(px(screen_x))
                            .top(px(screen_y))
                            .w(px(width))
                            .h(px(height))
                            .rounded(px((10.0 * zoom).clamp(4.0, 14.0)))
                            .border_2()
                            .border_color(if is_selected {
                                theme.primary
                            } else {
                                theme.transparent
                            })
                            .bg(theme.transparent)
                            .overflow_hidden()
                            .child(img(image).w_full().h_full())
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, _, _, cx| {
                                    this.select_node_from_pointer(node_id);
                                    cx.notify();
                                }),
                            )
                            .context_menu(node_context_menu(
                                menu_view.clone(),
                                node_id,
                                menu_kind,
                                self.settings.language,
                            ))
                            .into_any_element();
                    }
                }

                v_flex()
                    .id(node_element_id.clone())
                    .absolute()
                    .left(px(screen_x))
                    .top(px(screen_y))
                    .w(px(width))
                    .h(px(height))
                    .rounded(px((10.0 * zoom).clamp(4.0, 14.0)))
                    .border_2()
                    .border_color(border_color)
                    .bg(card_bg)
                    .text_color(card_fg)
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(px(header_height))
                            .w_full()
                            .px(px(padding))
                            .gap_1()
                            .border_b_1()
                            .border_color(theme.border)
                            .bg(if is_note {
                                theme.primary.opacity(0.10)
                            } else {
                                theme.muted.opacity(0.35)
                            })
                            .child(
                                Icon::new(if is_note {
                                    IconName::FileText
                                } else {
                                    IconName::StickyNote
                                })
                                .size(px((13.0 * zoom).clamp(7.0, 18.))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_size(px((12.0 * zoom).clamp(6.0, 18.)))
                                    .font_bold()
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px((10.0 * zoom).clamp(5.0, 14.)))
                                    .text_color(theme.muted_foreground)
                                    .child(format.short_label()),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .overflow_hidden()
                            .p(px(padding))
                            .text_size(px(font_size))
                            .child(render_text_content(node.id, content, format, zoom)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.select_node_from_pointer(node_id);
                            cx.notify();
                        }),
                    )
                    .context_menu(node_context_menu(
                        menu_view.clone(),
                        node_id,
                        menu_kind,
                        self.settings.language,
                    ))
                    .into_any_element()
            }
            NodeKind::Mind { title, parent } => {
                let root = parent.is_none();
                let icon_size = (15.0 * zoom).clamp(7.0, 20.0);
                let bg = if root {
                    theme.primary.opacity(0.14)
                } else {
                    theme.popover
                };
                let accent = if root { theme.primary } else { theme.border };

                h_flex()
                    .id(node_element_id.clone())
                    .absolute()
                    .left(px(screen_x))
                    .top(px(screen_y))
                    .w(px(width))
                    .h(px(height))
                    .px(px(padding))
                    .gap_2()
                    .rounded(px((28.0 * zoom).clamp(8.0, 30.0)))
                    .border_2()
                    .border_color(if is_selected { theme.primary } else { accent })
                    .bg(bg)
                    .text_color(card_fg)
                    .overflow_hidden()
                    .child(
                        Icon::new(IconName::Brain)
                            .size(px(icon_size))
                            .text_color(theme.primary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px((MIND_FONT_SIZE * zoom).clamp(6.0, 20.)))
                            .line_height(relative(1.45))
                            .whitespace_normal()
                            .font_bold()
                            .child(title.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.select_node_from_pointer(node_id);
                            cx.notify();
                        }),
                    )
                    .context_menu(node_context_menu(
                        menu_view.clone(),
                        node_id,
                        NodeMenuKind::Mind,
                        self.settings.language,
                    ))
                    .into_any_element()
            }
            NodeKind::Image { path, .. } => {
                let full_path = self.root.join(path);
                if rotated {
                    let key = rotated_svg_key(node, "", Some(path), None, &theme);
                    let image = self.cached_rotated_svg(key, || {
                        build_image_svg(node.w, node.h, &full_path, node.rotation, &theme)
                    });
                    if let Some(image) = image {
                        return base
                            .p(px(0.))
                            .child(img(image).w_full().h_full())
                            .into_any_element();
                    }
                }

                if full_path.exists() {
                    base.p(px((4.0 * zoom).clamp(1.0, 6.)))
                        .child(
                            img(full_path)
                                .w_full()
                                .h_full()
                                .object_fit(ObjectFit::Contain),
                        )
                        .into_any_element()
                } else {
                    base.flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.muted_foreground)
                        .text_size(px(font_size))
                        .child(self.tr("图片缺失", "Image missing"))
                        .into_any_element()
                }
            }
        }
    }

    fn render_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let background = cx.theme().background;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let popover = cx.theme().popover;
        let popover_fg = cx.theme().popover_foreground;
        let primary = cx.theme().primary;
        let language = self.settings.language;
        let dark = self.settings.theme.dark;
        let root_display = self.root.display().to_string();

        let section = |title: String, description: String, body: AnyElement| {
            v_flex()
                .gap_3()
                .p_5()
                .rounded(px(12.))
                .border_1()
                .border_color(border)
                .bg(popover)
                .text_color(popover_fg)
                .child(div().text_lg().font_bold().child(title))
                .child(div().text_sm().text_color(muted).child(description))
                .child(body)
                .into_any_element()
        };

        let language_body = h_flex()
            .gap_2()
            .child(
                Button::new("language-zh")
                    .label("中文")
                    .selected(language == Language::Zh)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_language(Language::Zh, window, cx);
                    })),
            )
            .child(
                Button::new("language-en")
                    .label("English")
                    .selected(language == Language::En)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_language(Language::En, window, cx);
                    })),
            )
            .into_any_element();

        let preset_buttons = THEME_PRESETS
            .iter()
            .map(|preset| {
                let selected = self.settings.theme.matches_preset(preset);
                let background_color = preset
                    .background
                    .and_then(accent_preview)
                    .unwrap_or(background);
                let accent_color = preset.accent.and_then(accent_preview).unwrap_or(primary);
                Button::new(SharedString::from(format!("theme-preset-{}", preset.id)))
                    .ghost()
                    .small()
                    .selected(selected)
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .size(px(13.))
                                    .rounded_full()
                                    .bg(background_color)
                                    .border_1()
                                    .border_color(border),
                            )
                            .child(div().size(px(9.)).rounded_full().bg(accent_color))
                            .child(div().child(preset.name)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_theme_preset(preset, window, cx);
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let theme_body = v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("theme-light")
                            .label(self.tr("浅色", "Light"))
                            .selected(!dark)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_theme_dark(false, cx);
                            })),
                    )
                    .child(
                        Button::new("theme-dark")
                            .label(self.tr("深色", "Dark"))
                            .selected(dark)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_theme_dark(true, cx);
                            })),
                    ),
            )
            .child(div().text_sm().child(self.tr("主题预设", "Theme presets")))
            .child(h_flex().gap_2().flex_wrap().children(preset_buttons))
            .child(
                div()
                    .text_sm()
                    .child(self.tr("自定义主题颜色", "Custom theme colors")),
            )
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(78.))
                                    .text_sm()
                                    .text_color(muted)
                                    .child(self.tr("背景", "Background")),
                            )
                            .child(Input::new(&self.background_input).w(px(132.))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(78.))
                                    .text_sm()
                                    .text_color(muted)
                                    .child(self.tr("前景", "Foreground")),
                            )
                            .child(Input::new(&self.foreground_input).w(px(132.))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(78.))
                                    .text_sm()
                                    .text_color(muted)
                                    .child(self.tr("侧边栏", "Sidebar")),
                            )
                            .child(Input::new(&self.sidebar_input).w(px(132.))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(78.))
                                    .text_sm()
                                    .text_color(muted)
                                    .child(self.tr("边框", "Border")),
                            )
                            .child(Input::new(&self.border_input).w(px(132.))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(78.))
                                    .text_sm()
                                    .text_color(muted)
                                    .child(self.tr("主题色", "Accent")),
                            )
                            .child(Input::new(&self.accent_input).w(px(132.))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("apply-custom-theme")
                            .primary()
                            .label(self.tr("应用自定义主题", "Apply custom theme"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_custom_theme(window, cx);
                            })),
                    )
                    .child(
                        Button::new("reset-theme")
                            .ghost()
                            .label(self.tr("恢复默认", "Reset"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reset_theme(window, cx);
                            })),
                    ),
            )
            .into_any_element();

        let storage_body = v_flex()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .rounded(px(8.))
                    .bg(muted.opacity(0.12))
                    .p_3()
                    .text_sm()
                    .truncate()
                    .child(root_display),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("choose-root")
                            .primary()
                            .icon(IconName::FolderOpen)
                            .label(self.tr("选择文件夹…", "Choose folder..."))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_notes_folder(window, cx);
                            })),
                    )
                    .child(
                        Button::new("reset-root")
                            .ghost()
                            .label(self.tr("恢复默认路径", "Reset to default"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_notes_root(set_notes_root_default(), window, cx);
                            })),
                    ),
            )
            .into_any_element();

        v_flex()
            .size_full()
            .bg(background)
            .child(
                h_flex()
                    .h(px(56.))
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(border)
                    .child(Icon::new(IconName::Settings).size(px(20.)).text_color(primary))
                    .child(div().text_lg().font_bold().child(self.tr("设置", "Settings")))
                    .child(div().flex_1())
                    .child(
                        Button::new("settings-toggle-sidebar")
                            .ghost()
                            .small()
                            .icon(if self.sidebar_collapsed {
                                IconName::PanelLeftOpen
                            } else {
                                IconName::PanelLeftClose
                            })
                            .tooltip(if self.sidebar_collapsed {
                                self.tr("展开侧边栏", "Expand sidebar")
                            } else {
                                self.tr("收起侧边栏", "Collapse sidebar")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_sidebar(cx);
                            })),
                    )
                    .child(
                        Button::new("close-settings")
                            .ghost()
                            .small()
                            .icon(IconName::X)
                            .label(self.tr("返回", "Back"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .items_center()
                            .p_6()
                            .child(
                                v_flex()
                                    .w(px(920.))
                                    .max_w(relative(0.94))
                                    .gap_4()
                                    .child(section(
                                        self.tr("语言", "Language").to_string(),
                                        self.tr(
                                            "选择界面显示语言，切换后立即生效。",
                                            "Choose the interface language. Changes apply immediately.",
                                        )
                                        .to_string(),
                                        language_body,
                                    ))
                                    .child(section(
                                        self.tr("主题", "Theme").to_string(),
                                        self.tr(
                                            "选择浅色或深色模式，并使用预设或自定义主题色。",
                                            "Choose light or dark mode and use a preset or custom accent color.",
                                        )
                                        .to_string(),
                                        theme_body,
                                    ))
                                    .child(section(
                                        self.tr("笔记保存路径", "Notes storage").to_string(),
                                        self.tr(
                                            "所有笔记、白板数据和粘贴的图片都会保存在此目录。",
                                            "All notes, board data and pasted images are stored in this folder.",
                                        )
                                        .to_string(),
                                        storage_body,
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_board(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.note.is_none() {
            let muted = cx.theme().muted_foreground;
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(cx.theme().background)
                .child(
                    Icon::new(IconName::NotebookPen)
                        .size(px(44.))
                        .text_color(muted),
                )
                .child(
                    div()
                        .text_lg()
                        .font_bold()
                        .child(self.tr("还没有打开笔记", "No note is open")),
                )
                .child(div().text_sm().text_color(muted).child(self.tr(
                    "从左侧选择一篇笔记，或新建一篇笔记开始。",
                    "Choose a note from the sidebar, or create a new one.",
                )))
                .child(
                    Button::new("empty-new-note")
                        .primary()
                        .label(self.tr("新建笔记", "New note"))
                        .icon(IconName::Plus)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_create_dialog(CreateMode::Markdown, window, cx);
                        })),
                )
                .into_any_element();
        }

        let status = self.status.clone();
        let zoom = self
            .note
            .as_ref()
            .map(|note| note.board.camera.zoom)
            .unwrap_or(1.0);
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;

        v_flex()
            .size_full()
            .child(self.render_toolbar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_canvas(window, cx)),
            )
            .child(
                h_flex()
                    .h(px(30.))
                    .px_3()
                    .gap_3()
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted)
                    .child(div().flex_1().min_w_0().overflow_hidden().child(status))
                    .child(self.tr(
                        "拖动平移 · 滚轮缩放 · 双击编辑 · Shift+拖动框选思维导图 · 右键节点菜单 · Ctrl+V 粘贴图片",
                        "Drag to pan · wheel to zoom · double-click to edit · Shift+drag to box-select mind maps · right-click for menu · Ctrl+V to paste",
                    ))
                    .child(if self.is_zh() {
                        format!("缩放 {}%", (zoom * 100.0).round())
                    } else {
                        format!("Zoom {}%", (zoom * 100.0).round())
                    }),
            )
            .into_any_element()
    }

    fn render_create_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.create_dialog.as_ref() else {
            return div().into_any_element();
        };
        let mode = dialog.mode;
        let overlay = cx.theme().overlay;
        let popover = cx.theme().popover;
        let popover_fg = cx.theme().popover_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let primary = cx.theme().primary;

        let is_note = mode.is_note();
        let format = mode.format();

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(overlay)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(Self::create_key_down))
            .child(
                v_flex()
                    .w(px(430.))
                    .gap_4()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .text_color(popover_fg)
                    .shadow_lg()
                    .p_5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(if is_note {
                                    IconName::FilePlus
                                } else {
                                    IconName::FolderPlus
                                })
                                .size(px(20.))
                                .text_color(primary),
                            )
                            .child(div().text_lg().font_bold().child(if is_note {
                                self.tr("新建笔记", "New note")
                            } else {
                                self.tr("新建目录", "New folder")
                            })),
                    )
                    .child(div().text_sm().text_color(muted).child(if is_note {
                        self.tr(
                            "名称将作为文件名，留空则使用默认名称。",
                            "The name becomes the file name. Leave it blank to use the default.",
                        )
                    } else {
                        self.tr(
                            "目录创建后可在左侧继续新建笔记。",
                            "Create notes inside the folder from the sidebar.",
                        )
                    }))
                    .child(Input::new(&self.create_input).w_full())
                    .when(is_note, |this| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(self.tr("文本格式", "Text format")),
                                )
                                .child(
                                    Button::new("create-format-md")
                                        .small()
                                        .label("Markdown")
                                        .selected(format == TextFormat::Markdown)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(dialog) = &mut this.create_dialog {
                                                dialog.mode = CreateMode::Markdown;
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("create-format-txt")
                                        .small()
                                        .label(self.tr("纯文本", "Plain text"))
                                        .selected(format == TextFormat::Plain)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(dialog) = &mut this.create_dialog {
                                                dialog.mode = CreateMode::Plain;
                                            }
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("create-cancel")
                                    .ghost()
                                    .label(self.tr("取消", "Cancel"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_dialog = None;
                                        this.board_focus.focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("create-confirm")
                                    .primary()
                                    .label(self.tr("创建", "Create"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_create(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn create_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            self.create_dialog = None;
            self.board_focus.focus(window, cx);
            cx.notify();
        } else if event.keystroke.key == "enter" {
            self.confirm_create(window, cx);
        }
    }

    fn render_rename_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.rename_dialog.as_ref() else {
            return div().into_any_element();
        };
        let is_dir = dialog.is_dir;
        let old_name = dialog
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(self.tr("未命名", "Untitled"))
            .to_string();
        let overlay = cx.theme().overlay;
        let popover = cx.theme().popover;
        let popover_fg = cx.theme().popover_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let primary = cx.theme().primary;

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(overlay)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(Self::rename_key_down))
            .child(
                v_flex()
                    .w(px(460.))
                    .max_w(relative(0.92))
                    .gap_4()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .text_color(popover_fg)
                    .shadow_lg()
                    .p_5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(if is_dir {
                                    IconName::Folder
                                } else {
                                    IconName::FileText
                                })
                                .size(px(20.))
                                .text_color(primary),
                            )
                            .child(div().text_lg().font_bold().child(if is_dir {
                                self.tr("重命名目录", "Rename folder")
                            } else {
                                self.tr("重命名笔记", "Rename note")
                            })),
                    )
                    .child(div().text_sm().text_color(muted).child(if self.is_zh() {
                        format!("当前名称：{old_name}")
                    } else {
                        format!("Current name: {old_name}")
                    }))
                    .child(Input::new(&self.rename_input).w_full())
                    .child(div().text_xs().text_color(muted).child(if is_dir {
                        self.tr(
                            "目录重命名后，其中的笔记会随目录一起移动。",
                            "Notes inside the folder move with it.",
                        )
                    } else {
                        self.tr(
                            "文件扩展名会保留；白板数据也会同步重命名。",
                            "The file extension is kept and the board data is renamed too.",
                        )
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("rename-cancel")
                                    .ghost()
                                    .label(self.tr("取消", "Cancel"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.rename_dialog = None;
                                        this.board_focus.focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("rename-confirm")
                                    .primary()
                                    .label(self.tr("重命名", "Rename"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_rename(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_delete_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.delete_dialog.as_ref() else {
            return div().into_any_element();
        };
        let is_dir = dialog.is_dir;
        let name = dialog
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(self.tr("未命名", "Untitled"))
            .to_string();
        let overlay = cx.theme().overlay;
        let popover = cx.theme().popover;
        let popover_fg = cx.theme().popover_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(overlay)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(Self::delete_key_down))
            .child(
                v_flex()
                    .w(px(470.))
                    .max_w(relative(0.92))
                    .gap_4()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .text_color(popover_fg)
                    .shadow_lg()
                    .p_5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(IconName::Trash).size(px(20.)).text_color(danger))
                            .child(div().text_lg().font_bold().child(if is_dir {
                                self.tr("删除目录", "Delete folder")
                            } else {
                                self.tr("删除笔记", "Delete note")
                            })),
                    )
                    .child(div().text_sm().child(if self.is_zh() {
                        format!("确定要删除“{name}”吗？")
                    } else {
                        format!("Delete \"{name}\"?")
                    }))
                    .child(div().text_sm().text_color(muted).child(if is_dir {
                        self.tr(
                            "目录中的所有笔记、白板数据和图片都会被删除，此操作不可撤销。",
                            "All notes, board data and images in this folder will be deleted. This cannot be undone.",
                        )
                    } else {
                        self.tr(
                            "笔记文件及其白板数据会被删除，此操作不可撤销。",
                            "The note file and its board data will be deleted. This cannot be undone.",
                        )
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("delete-cancel")
                                    .ghost()
                                    .label(self.tr("取消", "Cancel"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_dialog = None;
                                        this.board_focus.focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("delete-confirm")
                                    .danger()
                                    .label(self.tr("删除", "Delete"))
                                    .icon(IconName::Trash)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_delete(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_editor_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = self.editor else {
            return div().into_any_element();
        };
        let overlay = cx.theme().overlay;
        let popover = cx.theme().popover;
        let popover_fg = cx.theme().popover_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let primary = cx.theme().primary;

        let (title, hint, is_body) = match editor.target {
            EditTarget::Note => (
                self.tr("编辑笔记", "Edit note"),
                self.tr(
                    "支持 Markdown 语法，保存后会同步到左侧文件。",
                    "Markdown is supported. Saving updates the file in the sidebar.",
                ),
                true,
            ),
            EditTarget::Text(_) => (
                self.tr("编辑文本卡片", "Edit text card"),
                self.tr(
                    "双击卡片或点击保存按钮保存。",
                    "Double-click a card or use the Save button.",
                ),
                true,
            ),
            EditTarget::Mind(_) => (
                self.tr("编辑思维导图节点", "Edit mind map node"),
                self.tr(
                    "修改节点标题，按 Enter 保存。",
                    "Edit the node title and press Enter to save.",
                ),
                false,
            ),
        };

        let show_format = matches!(editor.target, EditTarget::Text(_));
        let viewport = window.viewport_size();
        let max_width = (f32::from(viewport.width) - 40.0).max(480.0);
        let max_height = (f32::from(viewport.height) - 40.0).max(320.0);
        if is_body {
            self.editor_width = self.editor_width.clamp(480.0, max_width);
            self.editor_height = self.editor_height.clamp(320.0, max_height);
        }
        let dialog_width = if is_body { self.editor_width } else { 760.0 };

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(overlay)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_move(cx.listener(Self::editor_resize_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::finish_editor_resize))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(Self::editor_key_down))
            .child(
                v_flex()
                    .relative()
                    .w(px(dialog_width))
                    .max_w(relative(0.92))
                    .when(is_body, |this| this.h(px(self.editor_height)))
                    .gap_3()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .text_color(popover_fg)
                    .shadow_lg()
                    .p_5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(if is_body {
                                    IconName::SquarePen
                                } else {
                                    IconName::Brain
                                })
                                .size(px(20.))
                                .text_color(primary),
                            )
                            .child(div().flex_1().text_lg().font_bold().child(title))
                            .when(show_format, |this| {
                                this.child(
                                    Button::new("edit-format-md")
                                        .small()
                                        .label("Markdown")
                                        .selected(editor.format == TextFormat::Markdown)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(editor) = &mut this.editor {
                                                editor.format = TextFormat::Markdown;
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("edit-format-txt")
                                        .small()
                                        .label(self.tr("纯文本", "Plain text"))
                                        .selected(editor.format == TextFormat::Plain)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(editor) = &mut this.editor {
                                                editor.format = TextFormat::Plain;
                                            }
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .child(div().text_sm().text_color(muted).child(hint))
                    .child(if is_body {
                        Textarea::new(&self.body_input)
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .into_any_element()
                    } else {
                        Input::new(&self.title_input).w_full().into_any_element()
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("editor-cancel")
                                    .ghost()
                                    .label(self.tr("取消", "Cancel"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_editor(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("editor-save")
                                    .primary()
                                    .label(self.tr("保存", "Save"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_editor(window, cx);
                                    })),
                            ),
                    )
                    .when(is_body, |this| {
                        this.child(
                            div()
                                .id("editor-resize")
                                .absolute()
                                .right(px(4.))
                                .bottom(px(4.))
                                .flex()
                                .size(px(24.))
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .cursor_nwse_resize()
                                .hover(|style| style.bg(muted.opacity(0.15)))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(Self::start_editor_resize),
                                )
                                .child(
                                    Icon::new(IconName::ResizeCorner)
                                        .size(px(16.))
                                        .text_color(muted),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for WhiteboardApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = if self.sidebar_collapsed {
            None
        } else {
            Some(self.render_sidebar(window, cx))
        };
        let board = if self.settings_open {
            self.render_settings(window, cx)
        } else {
            self.render_board(window, cx)
        };
        let create_dialog = self
            .create_dialog
            .is_some()
            .then(|| self.render_create_dialog(window, cx));
        let editor_dialog = self
            .editor
            .is_some()
            .then(|| self.render_editor_dialog(window, cx));
        let rename_dialog = self
            .rename_dialog
            .is_some()
            .then(|| self.render_rename_dialog(window, cx));
        let delete_dialog = self
            .delete_dialog
            .is_some()
            .then(|| self.render_delete_dialog(window, cx));

        let split = if let Some(sidebar) = sidebar {
            h_resizable(SharedString::from(format!(
                "main-split-{}",
                self.sidebar_layout_id
            )))
            .child(resizable_panel().size(px(286.)).flex_none().child(sidebar))
            .child(resizable_panel().child(board))
            .into_any_element()
        } else {
            div().size_full().child(board).into_any_element()
        };

        let mut content = v_flex().size_full();
        if CUSTOM_TITLEBAR {
            content = content.child(self.render_custom_titlebar(window, cx));
        }
        let content = content.child(split).into_any_element();

        div()
            .relative()
            .size_full()
            .child(content)
            .children(create_dialog)
            .children(editor_dialog)
            .children(rename_dialog)
            .children(delete_dialog)
    }
}

fn rotated_svg_key(
    node: &BoardNode,
    title: &str,
    content: Option<&str>,
    format: Option<TextFormat>,
    theme: &Theme,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    node.id.hash(&mut hasher);
    node.rotation.to_bits().hash(&mut hasher);
    node.w.to_bits().hash(&mut hasher);
    node.h.to_bits().hash(&mut hasher);
    title.hash(&mut hasher);
    content.hash(&mut hasher);
    format.map(TextFormat::extension).hash(&mut hasher);
    theme.is_dark().hash(&mut hasher);
    theme.primary.hash(&mut hasher);
    theme.popover.hash(&mut hasher);
    theme.popover_foreground.hash(&mut hasher);
    theme.border.hash(&mut hasher);
    theme.muted.hash(&mut hasher);
    theme.muted_foreground.hash(&mut hasher);
    theme.font_family.hash(&mut hasher);
    hasher.finish()
}

fn css_color(color: Hsla) -> String {
    let rgba = color.to_rgb();
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn strip_inline_markdown(value: &str) -> String {
    let mut output = String::new();
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' | '_' | '`' | '~' => {
                index += 1;
            }
            '[' => {
                let label_start = index + 1;
                let mut label_end = label_start;
                while label_end < chars.len() && chars[label_end] != ']' {
                    label_end += 1;
                }
                if label_end + 1 < chars.len() && chars[label_end + 1] == '(' {
                    let mut link_end = label_end + 2;
                    while link_end < chars.len() && chars[link_end] != ')' {
                        link_end += 1;
                    }
                    output.extend(chars[label_start..label_end].iter());
                    index = (link_end + 1).min(chars.len());
                } else {
                    output.push(chars[index]);
                    index += 1;
                }
            }
            character => {
                output.push(character);
                index += 1;
            }
        }
    }
    output
}

fn wrap_svg_line(value: &str, max_width: f32, font_size: f32) -> Vec<String> {
    let max_units = ((max_width / font_size.max(6.0)) * 2.0).max(6.0) as usize;
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut units = 0;

    for character in value.chars() {
        let width = if character.is_ascii() { 1 } else { 2 };
        if units + width > max_units && !current.is_empty() {
            lines.push(current);
            current = String::new();
            units = 0;
        }
        current.push(character);
        units += width;
    }

    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn svg_text_lines(
    content: &str,
    format: TextFormat,
    max_width: f32,
    base_size: f32,
) -> Vec<(String, f32, bool)> {
    let mut lines = Vec::new();
    let mut in_code = false;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim_start();
        if format == TextFormat::Markdown && trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }

        if trimmed.is_empty() {
            lines.push((String::new(), base_size, false));
            continue;
        }

        let mut text = raw_line.trim_end().to_string();
        let mut size = base_size;
        let mut bold = false;

        if format == TextFormat::Markdown && !in_code {
            let heading_level = trimmed
                .chars()
                .take_while(|character| *character == '#')
                .count();
            if heading_level > 0 && trimmed.chars().nth(heading_level) == Some(' ') {
                text = trimmed[heading_level + 1..].to_string();
                size = base_size + (7 - heading_level.min(6)) as f32 * 1.5;
                bold = true;
            } else if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "))
            {
                text = format!("• {rest}");
            }
            text = strip_inline_markdown(&text);
        }

        for (index, part) in wrap_svg_line(&text, max_width, size)
            .into_iter()
            .enumerate()
        {
            lines.push((part, size, bold && index == 0));
        }
    }

    lines
}

fn svg_fit_transform(width: f32, height: f32, rotation: f32) -> (f32, f32, f32) {
    let angle = rotation.to_radians();
    let (sin, cos) = angle.sin_cos();
    let sin = sin.abs();
    let cos = cos.abs();
    let bounds_width = width * cos + height * sin;
    let bounds_height = width * sin + height * cos;
    let scale = (width / bounds_width.max(1.0))
        .min(height / bounds_height.max(1.0))
        .min(1.0);
    (scale, width / 2.0, height / 2.0)
}

#[allow(clippy::too_many_arguments)]
fn build_text_svg(
    width: f32,
    height: f32,
    title: &str,
    content: &str,
    format: TextFormat,
    rotation: f32,
    theme: &Theme,
    is_note: bool,
) -> String {
    let width = width.max(120.0);
    let height = height.max(80.0);
    let bg = css_color(theme.popover);
    let fg = css_color(theme.popover_foreground);
    let border = css_color(theme.border);
    let muted = css_color(theme.muted);
    let header_height = 32.0;
    let padding = 12.0;
    let font_size = 13.0;
    let header_opacity = if is_note { 0.14 } else { 0.22 };
    let (scale, cx, cy) = svg_fit_transform(width, height, rotation);

    let mut body = String::new();
    let mut y = header_height + padding + font_size;
    for (line, size, bold) in svg_text_lines(content, format, width - padding * 2.0, font_size) {
        if y > height - padding {
            break;
        }
        if line.is_empty() {
            y += size * 0.6;
            continue;
        }
        let weight = if bold { "bold" } else { "normal" };
        body.push_str(&format!(
            r#"<text x="{padding:.2}" y="{y:.2}" font-family="sans-serif" font-size="{size:.2}" font-weight="{weight}" fill="{fg}">{}</text>"#,
            xml_escape(&line)
        ));
        y += size * 1.45;
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.2}" height="{height:.2}" viewBox="0 0 {width:.2} {height:.2}">
  <g transform="translate({cx:.3} {cy:.3}) scale({scale:.4}) rotate({rotation:.3}) translate({neg_cx:.3} {neg_cy:.3})">
    <rect x="1" y="1" width="{rect_w:.2}" height="{rect_h:.2}" rx="10" fill="{bg}" stroke="{border}" stroke-width="2"/>
    <rect x="1" y="1" width="{rect_w:.2}" height="{header_height:.2}" rx="10" fill="{muted}" fill-opacity="{header_opacity:.2}"/>
    <text x="{padding:.2}" y="{title_y:.2}" font-family="sans-serif" font-size="12" font-weight="bold" fill="{fg}">{title}</text>
    <line x1="1" y1="{header_height:.2}" x2="{line_x:.2}" y2="{header_height:.2}" stroke="{border}" stroke-width="1"/>
    {body}
  </g>
</svg>"#,
        width = width,
        height = height,
        cx = cx,
        cy = cy,
        scale = scale,
        rotation = rotation,
        neg_cx = -cx,
        neg_cy = -cy,
        rect_w = width - 2.0,
        rect_h = height - 2.0,
        header_height = header_height,
        header_opacity = header_opacity,
        padding = padding,
        title_y = header_height / 2.0 + 4.0,
        title = xml_escape(title),
        line_x = width - 1.0,
        muted = muted,
        fg = fg,
        bg = bg,
        border = border,
        body = body,
    )
}

fn build_image_svg(
    width: f32,
    height: f32,
    path: &Path,
    rotation: f32,
    theme: &Theme,
) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let encoded = BASE64.encode(bytes);
    let width = width.max(80.0);
    let height = height.max(60.0);
    let bg = css_color(theme.popover);
    let border = css_color(theme.border);
    let (scale, cx, cy) = svg_fit_transform(width, height, rotation);
    let padding = 4.0;

    Some(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.2}" height="{height:.2}" viewBox="0 0 {width:.2} {height:.2}">
  <g transform="translate({cx:.3} {cy:.3}) scale({scale:.4}) rotate({rotation:.3}) translate({neg_cx:.3} {neg_cy:.3})">
    <rect x="1" y="1" width="{rect_w:.2}" height="{rect_h:.2}" rx="8" fill="{bg}" stroke="{border}" stroke-width="2"/>
    <image x="{padding:.2}" y="{padding:.2}" width="{image_w:.2}" height="{image_h:.2}" preserveAspectRatio="xMidYMid meet" href="data:image/png;base64,{encoded}"/>
  </g>
</svg>"#,
        width = width,
        height = height,
        cx = cx,
        cy = cy,
        scale = scale,
        rotation = rotation,
        neg_cx = -cx,
        neg_cy = -cy,
        rect_w = width - 2.0,
        rect_h = height - 2.0,
        padding = padding,
        image_w = width - padding * 2.0,
        image_h = height - padding * 2.0,
        bg = bg,
        border = border,
        encoded = encoded,
    ))
}

fn node_context_menu(
    view: Entity<WhiteboardApp>,
    node_id: Uuid,
    kind: NodeMenuKind,
    language: Language,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, window, _cx| {
        let mut menu = menu;

        if !matches!(kind, NodeMenuKind::Image) {
            let edit_view = view.clone();
            menu = menu.item(
                PopupMenuItem::new(language.pick("编辑", "Edit"))
                    .icon(IconName::Pencil)
                    .on_click(window.listener_for(&edit_view, move |this, _, window, cx| {
                        this.open_editor_for_node(node_id, window, cx);
                    })),
            );
        }

        if let NodeMenuKind::Text(format) = kind {
            let label = match format {
                TextFormat::Markdown => language.pick("切换为纯文本", "Switch to plain text"),
                TextFormat::Plain => language.pick("切换为 Markdown", "Switch to Markdown"),
            };
            let format_view = view.clone();
            menu = menu.item(PopupMenuItem::new(label).icon(IconName::Type).on_click(
                window.listener_for(&format_view, move |this, _, _, cx| {
                    this.toggle_node_format(node_id, cx);
                }),
            ));
        }

        if matches!(
            kind,
            NodeMenuKind::Note | NodeMenuKind::Text(_) | NodeMenuKind::Image
        ) {
            let rotate_cw_view = view.clone();
            menu = menu.item(
                PopupMenuItem::new(language.pick("顺时针旋转 15°", "Rotate 15° clockwise"))
                    .icon(IconName::RotateCw)
                    .on_click(window.listener_for(&rotate_cw_view, move |this, _, _, cx| {
                        this.rotate_node(node_id, 15.0, cx);
                    })),
            );
            let rotate_ccw_view = view.clone();
            menu = menu.item(
                PopupMenuItem::new(language.pick("逆时针旋转 15°", "Rotate 15° counterclockwise"))
                    .icon(IconName::RotateCcw)
                    .on_click(
                        window.listener_for(&rotate_ccw_view, move |this, _, _, cx| {
                            this.rotate_node(node_id, -15.0, cx);
                        }),
                    ),
            );
            let reset_rotation_view = view.clone();
            menu = menu.item(
                PopupMenuItem::new(language.pick("重置旋转", "Reset rotation"))
                    .icon(IconName::RefreshCw)
                    .on_click(
                        window.listener_for(&reset_rotation_view, move |this, _, _, cx| {
                            this.set_node_rotation(node_id, 0.0, cx);
                        }),
                    ),
            );
        }

        if matches!(kind, NodeMenuKind::Mind) {
            let child_view = view.clone();
            menu = menu.item(
                PopupMenuItem::new(language.pick("添加子节点", "Add child node"))
                    .icon(IconName::GitBranch)
                    .on_click(
                        window.listener_for(&child_view, move |this, _, window, cx| {
                            this.select_node_from_pointer(node_id);
                            this.add_mind_child(window, cx);
                        }),
                    ),
            );
        }

        if !matches!(kind, NodeMenuKind::Image) {
            menu = menu.separator();
        }
        let can_delete = !matches!(kind, NodeMenuKind::Note);
        let delete_view = view.clone();
        menu.item(
            PopupMenuItem::new(if can_delete {
                language.pick("删除节点", "Delete node")
            } else {
                language.pick("笔记主卡片不能删除", "The main note card cannot be deleted")
            })
            .icon(IconName::Trash)
            .disabled(!can_delete)
            .on_click(window.listener_for(
                &delete_view,
                move |this, _, window, cx| {
                    this.select_node_from_pointer(node_id);
                    this.delete_selected(window, cx);
                },
            )),
        )
    }
}

fn find_free_node_position(
    nodes: &[BoardNode],
    desired_x: f32,
    desired_y: f32,
    width: f32,
    height: f32,
) -> (f32, f32) {
    const MARGIN: f32 = 18.0;
    let step = height + 24.0;

    for index in 0usize..=200 {
        let offset = if index == 0 {
            0.0
        } else {
            let distance = index.div_ceil(2) as f32 * step;
            if index % 2 == 1 { distance } else { -distance }
        };
        let y = desired_y + offset;
        let overlaps = nodes.iter().any(|node| {
            desired_x < node.x + node.w + MARGIN
                && desired_x + width + MARGIN > node.x
                && y < node.y + node.h + MARGIN
                && y + height + MARGIN > node.y
        });
        if !overlaps {
            return (desired_x, y);
        }
    }

    (desired_x, desired_y)
}

fn rects_overlap(a: &BoardNode, b: &BoardNode, margin: f32) -> bool {
    a.x < b.x + b.w + margin
        && a.x + a.w + margin > b.x
        && a.y < b.y + b.h + margin
        && a.y + a.h + margin > b.y
}

fn is_mind_ancestor(nodes: &[BoardNode], ancestor: Uuid, mut node_id: Uuid) -> bool {
    for _ in 0..1024 {
        let Some(node) = nodes.iter().find(|node| node.id == node_id) else {
            return false;
        };
        let NodeKind::Mind {
            parent: Some(parent),
            ..
        } = &node.kind
        else {
            return false;
        };
        if *parent == ancestor {
            return true;
        }
        node_id = *parent;
    }
    false
}

fn mind_subtree_ids(nodes: &[BoardNode], root_id: Uuid) -> HashSet<Uuid> {
    let mut ids = HashSet::new();
    ids.insert(root_id);

    loop {
        let mut added = false;
        for node in nodes {
            if let NodeKind::Mind {
                parent: Some(parent),
                ..
            } = &node.kind
                && ids.contains(parent)
                && !ids.contains(&node.id)
            {
                ids.insert(node.id);
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    ids
}

fn shift_mind_subtree(nodes: &mut [BoardNode], root_id: Uuid, delta_y: f32) {
    let ids = mind_subtree_ids(nodes, root_id);
    for node in nodes {
        if ids.contains(&node.id) {
            node.y += delta_y;
        }
    }
}

fn resolve_mind_overlaps(nodes: &mut [BoardNode]) {
    const GAP: f32 = 22.0;

    for _ in 0..512 {
        let mut shifted = false;

        'outer: for i in 0..nodes.len() {
            if !nodes[i].kind.is_mind() {
                continue;
            }
            for j in (i + 1)..nodes.len() {
                if !nodes[j].kind.is_mind()
                    || !rects_overlap(&nodes[i], &nodes[j], 12.0)
                    || is_mind_ancestor(nodes, nodes[i].id, nodes[j].id)
                    || is_mind_ancestor(nodes, nodes[j].id, nodes[i].id)
                {
                    continue;
                }

                let (lower, upper) =
                    if nodes[i].y > nodes[j].y || (nodes[i].y == nodes[j].y && i > j) {
                        (i, j)
                    } else {
                        (j, i)
                    };
                let delta = nodes[upper].y + nodes[upper].h + GAP - nodes[lower].y;
                let lower_id = nodes[lower].id;
                shift_mind_subtree(nodes, lower_id, delta);
                shifted = true;
                break 'outer;
            }
        }

        if !shifted {
            break;
        }
    }
}

fn remap_path(path: &Path, old: &Path, new: &Path) -> PathBuf {
    match path.strip_prefix(old) {
        Ok(relative) => new.join(relative),
        Err(_) => path.to_path_buf(),
    }
}

fn fit_image(width: f32, height: f32) -> (f32, f32) {
    let max_width = 520.0;
    let max_height = 420.0;
    let scale = (max_width / width.max(1.0))
        .min(max_height / height.max(1.0))
        .min(1.0);
    ((width * scale).max(80.0), (height * scale).max(60.0))
}

fn render_text_content(
    node_id: Uuid,
    content: String,
    format: TextFormat,
    zoom: f32,
) -> AnyElement {
    match format {
        TextFormat::Markdown => {
            TextView::markdown(SharedString::from(format!("markdown-{node_id}")), content)
                .selectable(false)
                .w_full()
                .min_w_0()
                .into_any_element()
        }
        TextFormat::Plain => {
            let font_size = (13.0 * zoom).clamp(5.0, 26.0);
            let line_height = (18.0 * zoom).clamp(6.0, 34.0);
            let lines = if content.is_empty() {
                vec![" ".to_string()]
            } else {
                content
                    .lines()
                    .map(|line| {
                        if line.is_empty() {
                            " ".to_string()
                        } else {
                            line.to_string()
                        }
                    })
                    .collect()
            };

            v_flex()
                .w_full()
                .gap_0()
                .children(lines.into_iter().map(|line| {
                    div()
                        .w_full()
                        .min_h(px(line_height))
                        .text_size(px(font_size))
                        .child(line)
                }))
                .into_any_element()
        }
    }
}

fn measured_mind_lines(
    title: &str,
    node_width: f32,
    window: &Window,
    font_family: &SharedString,
) -> usize {
    let run = TextRun {
        len: title.len(),
        font: font(font_family.clone()).bold(),
        color: Hsla::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_text(
            SharedString::from(title.to_string()),
            px(MIND_FONT_SIZE),
            &[run],
            Some(px(mind_text_width(node_width))),
            None,
        )
        .map(|lines| {
            lines
                .iter()
                .map(|line| line.wrap_boundaries().len() + 1)
                .sum()
        })
        .unwrap_or_else(|_| estimate_mind_lines(title, node_width))
        .max(1)
}

fn fit_mind_node_height(node: &mut BoardNode, window: &Window, font_family: &SharedString) -> bool {
    let NodeKind::Mind { title, .. } = &node.kind else {
        return false;
    };
    let height = mind_height_for_lines(measured_mind_lines(title, node.w, window, font_family));
    if (node.h - height).abs() > 0.01 {
        node.h = height;
        true
    } else {
        false
    }
}

fn fit_mind_node_heights(
    nodes: &mut [BoardNode],
    window: &Window,
    font_family: &SharedString,
) -> bool {
    let mut changed = false;
    for node in nodes.iter_mut().filter(|node| node.kind.is_mind()) {
        changed |= fit_mind_node_height(node, window, font_family);
    }
    changed
}

fn window_control_button(
    id: &'static str,
    area: WindowControlArea,
    icon: IconName,
    foreground: Hsla,
    hover_background: Hsla,
    hover_foreground: Hsla,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .h_full()
        .w(px(46.))
        .flex_shrink_0()
        .justify_center()
        .items_center()
        .text_color(foreground)
        .cursor_pointer()
        .window_control_area(area)
        .hover(move |style| style.bg(hover_background).text_color(hover_foreground))
        .child(Icon::new(icon).size(px(15.)))
        .into_any_element()
}

fn paint_grid(viewport: Bounds<Pixels>, camera: Camera, color: Hsla, window: &mut Window) {
    let mut spacing = 32.0 * camera.zoom;
    while spacing < 18.0 {
        spacing *= 2.0;
    }

    let start_x = camera.pan_x.rem_euclid(spacing);
    let start_y = camera.pan_y.rem_euclid(spacing);
    let dot = (2.0 * camera.zoom).clamp(1.0, 3.0);
    let dot_size = size(px(dot), px(dot));
    let width = f32::from(viewport.size.width);
    let height = f32::from(viewport.size.height);
    let dot_color = color.opacity(0.55);

    let mut x = start_x;
    while x < width {
        let mut y = start_y;
        while y < height {
            let dot_bounds = bounds(
                point(viewport.origin.x + px(x), viewport.origin.y + px(y)),
                dot_size,
            );
            window.paint_quad(fill(dot_bounds, dot_color));
            y += spacing;
        }
        x += spacing;
    }
}

fn paint_edges(
    viewport: Bounds<Pixels>,
    camera: Camera,
    edges: &[(f32, f32, f32, f32)],
    color: Hsla,
    window: &mut Window,
) {
    let stroke = px((2.0 * camera.zoom).clamp(1.0, 4.0));

    for (start_x, start_y, end_x, end_y) in edges {
        let (sx, sy) = camera.world_to_screen(*start_x, *start_y);
        let (ex, ey) = camera.world_to_screen(*end_x, *end_y);
        let start = point(viewport.origin.x + px(sx), viewport.origin.y + px(sy));
        let end = point(viewport.origin.x + px(ex), viewport.origin.y + px(ey));
        let dx = (end.x - start.x).max(px(40.0)) * 0.55;

        let mut path = PathBuilder::stroke(stroke);
        path.move_to(start);
        path.cubic_bezier_to(end, point(start.x + dx, start.y), point(end.x - dx, end.y));
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn mind_subtree_drag_includes_all_descendants() {
        let root = BoardNode::mind(0.0, 0.0, "Root", None);
        let root_id = root.id;
        let child = BoardNode::mind(300.0, 0.0, "Child", Some(root_id));
        let child_id = child.id;
        let grandchild = BoardNode::mind(600.0, 0.0, "Grandchild", Some(child_id));
        let grandchild_id = grandchild.id;
        let other = BoardNode::mind(300.0, 200.0, "Other", None);
        let other_id = other.id;

        let nodes = vec![root, child, grandchild, other];
        let ids = mind_subtree_ids(&nodes, root_id);
        assert!(ids.contains(&root_id));
        assert!(ids.contains(&child_id));
        assert!(ids.contains(&grandchild_id));
        assert!(!ids.contains(&other_id));
    }

    #[::core::prelude::v1::test]
    fn resolving_mind_overlaps_moves_conflicting_subtree() {
        let root = BoardNode::mind(0.0, 0.0, "Root", None);
        let root_id = root.id;
        let parent_a = BoardNode::mind(300.0, 0.0, "A", Some(root_id));
        let parent_a_id = parent_a.id;
        let parent_b = BoardNode::mind(300.0, 80.0, "B", Some(root_id));
        let parent_b_id = parent_b.id;
        let child_a = BoardNode::mind(600.0, 20.0, "A1", Some(parent_a_id));
        let child_b = BoardNode::mind(600.0, 20.0, "B1", Some(parent_b_id));

        let mut nodes = vec![root, parent_a, parent_b, child_a, child_b];
        resolve_mind_overlaps(&mut nodes);

        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                assert!(
                    !rects_overlap(&nodes[i], &nodes[j], 0.0),
                    "nodes still overlap: {:?} and {:?}",
                    nodes[i].id,
                    nodes[j].id
                );
            }
        }
    }
}
