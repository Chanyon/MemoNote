use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::Language;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    #[default]
    Markdown,
    Plain,
}

impl TextFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Plain => "txt",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Markdown => "MD",
            Self::Plain => "TXT",
        }
    }

    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "txt" => Self::Plain,
            _ => Self::Markdown,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Camera {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan_x: 48.0,
            pan_y: 48.0,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn world_to_screen(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.zoom + self.pan_x, y * self.zoom + self.pan_y)
    }

    pub fn screen_to_world(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.pan_x) / self.zoom, (y - self.pan_y) / self.zoom)
    }

    pub fn zoom_around(&mut self, screen_x: f32, screen_y: f32, next_zoom: f32) {
        let next_zoom = next_zoom.clamp(0.2, 3.0);
        let (world_x, world_y) = self.screen_to_world(screen_x, screen_y);
        self.zoom = next_zoom;
        self.pan_x = screen_x - world_x * next_zoom;
        self.pan_y = screen_y - world_y * next_zoom;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    Note {
        content: String,
        format: TextFormat,
    },
    Text {
        content: String,
        format: TextFormat,
    },
    Image {
        path: String,
        width: f32,
        height: f32,
    },
    Mind {
        title: String,
        parent: Option<Uuid>,
    },
}

impl NodeKind {
    pub fn is_note(&self) -> bool {
        matches!(self, Self::Note { .. })
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    pub fn is_mind(&self) -> bool {
        matches!(self, Self::Mind { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoardNode {
    pub id: Uuid,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub auto_height: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(flatten)]
    pub kind: NodeKind,
}

impl BoardNode {
    pub fn text(x: f32, y: f32, content: impl Into<String>, format: TextFormat) -> Self {
        Self {
            id: Uuid::new_v4(),
            x,
            y,
            w: 340.0,
            h: 210.0,
            rotation: 0.0,
            auto_height: false,
            locked: false,
            kind: NodeKind::Text {
                content: content.into(),
                format,
            },
        }
    }

    pub fn note(x: f32, y: f32, content: impl Into<String>, format: TextFormat) -> Self {
        Self {
            id: Uuid::new_v4(),
            x,
            y,
            w: 480.0,
            h: 350.0,
            rotation: 0.0,
            auto_height: false,
            locked: false,
            kind: NodeKind::Note {
                content: content.into(),
                format,
            },
        }
    }

    pub fn mind(x: f32, y: f32, title: impl Into<String>, parent: Option<Uuid>) -> Self {
        let mut node = Self {
            id: Uuid::new_v4(),
            x,
            y,
            w: 220.0,
            h: 58.0,
            rotation: 0.0,
            auto_height: false,
            locked: false,
            kind: NodeKind::Mind {
                title: title.into(),
                parent,
            },
        };
        fit_mind_node(&mut node);
        node
    }

    pub fn image(x: f32, y: f32, path: impl Into<String>, width: f32, height: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            x,
            y,
            w: width,
            h: height,
            rotation: 0.0,
            auto_height: false,
            locked: false,
            kind: NodeKind::Image {
                path: path.into(),
                width,
                height,
            },
        }
    }

    pub fn contains(&self, world_x: f32, world_y: f32) -> bool {
        world_x >= self.x
            && world_y >= self.y
            && world_x <= self.x + self.w
            && world_y <= self.y + self.h
    }
}

pub const MIND_FONT_SIZE: f32 = 13.0;
pub const MIND_LINE_HEIGHT: f32 = 19.0;
const MIND_MIN_HEIGHT: f32 = 58.0;
const MIND_HORIZONTAL_PADDING: f32 = 10.0;
const MIND_ICON_SIZE: f32 = 15.0;
const MIND_GAP: f32 = 8.0;
const MIND_BORDER: f32 = 4.0;

pub fn mind_text_width(node_width: f32) -> f32 {
    (node_width - MIND_HORIZONTAL_PADDING * 2.0 - MIND_ICON_SIZE - MIND_GAP - MIND_BORDER)
        .max(MIND_FONT_SIZE * 2.0)
}

pub fn mind_height_for_lines(lines: usize) -> f32 {
    (lines.max(1) as f32 * MIND_LINE_HEIGHT + MIND_HORIZONTAL_PADDING * 2.0 + MIND_BORDER + 2.0)
        .max(MIND_MIN_HEIGHT)
}

fn mind_char_width(ch: char) -> f32 {
    if ch.is_whitespace() {
        4.0
    } else if is_wide_char(ch) {
        MIND_FONT_SIZE
    } else if ch.is_ascii_uppercase() {
        8.5
    } else {
        7.0
    }
}

fn is_wide_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x115f
            | 0x2e80..=0xa4cf
            | 0xac00..=0xd7a3
            | 0xf900..=0xfaff
            | 0xfe10..=0xfe19
            | 0xfe30..=0xfe6f
            | 0xff00..=0xff60
            | 0xffe0..=0xffe6
            | 0x20000..=0x3fffd
    )
}

pub fn estimate_mind_lines(title: &str, node_width: f32) -> usize {
    let usable_width = (mind_text_width(node_width) * 0.94).max(MIND_FONT_SIZE);
    let mut lines = 0;
    for line in title.split('\n') {
        let mut count = 1;
        let mut width = 0.0;
        for ch in line.chars() {
            let char_width = mind_char_width(ch);
            if width > 0.0 && width + char_width > usable_width {
                count += 1;
                width = if ch.is_whitespace() { 0.0 } else { char_width };
            } else {
                width += char_width;
            }
        }
        lines += count;
    }
    lines.max(1)
}

pub fn fit_mind_node(node: &mut BoardNode) {
    if let NodeKind::Mind { title, .. } = &node.kind {
        node.h = mind_height_for_lines(estimate_mind_lines(title, node.w));
    }
}

pub fn repair_corrupted_mind_titles(nodes: &mut [BoardNode]) -> bool {
    let mut changed = false;
    for node in nodes.iter_mut() {
        let corrupted = matches!(
            &node.kind,
            NodeKind::Mind { title, .. }
                if !title.is_empty() && title.chars().all(|ch| ch == '\u{fffd}')
        );
        if !corrupted {
            continue;
        }
        if let NodeKind::Mind { title, parent } = &mut node.kind {
            *title = if parent.is_none() {
                "中心主题"
            } else {
                "子主题"
            }
            .to_string();
        }
        fit_mind_node(node);
        changed = true;
    }
    changed
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeConnection {
    pub id: Uuid,
    pub from: Uuid,
    pub to: Uuid,
}

impl NodeConnection {
    pub fn new(from: Uuid, to: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            from,
            to,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Board {
    #[serde(default)]
    pub camera: Camera,
    #[serde(default)]
    pub nodes: Vec<BoardNode>,
    #[serde(default)]
    pub connections: Vec<NodeConnection>,
    #[serde(default)]
    pub note_card_deleted: bool,
}

impl Board {
    pub fn with_note(content: impl Into<String>, format: TextFormat) -> Self {
        let content = content.into();
        Self {
            camera: Camera::default(),
            nodes: vec![BoardNode::note(96.0, 96.0, content, format)],
            connections: Vec::new(),
            note_card_deleted: false,
        }
    }

    pub fn note_node_mut(&mut self) -> Option<&mut BoardNode> {
        self.nodes.iter_mut().find(|node| node.kind.is_note())
    }

    pub fn node(&self, id: Uuid) -> Option<&BoardNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: Uuid) -> Option<&mut BoardNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    pub fn ensure_note_node(&mut self, content: &str, format: TextFormat) -> Uuid {
        if let Some(node) = self.note_node_mut() {
            node.kind = NodeKind::Note {
                content: content.to_string(),
                format,
            };
            return node.id;
        }

        if self.note_card_deleted {
            return Uuid::nil();
        }

        let node = BoardNode::note(96.0, 96.0, content, format);
        let id = node.id;
        self.nodes.insert(0, node);
        id
    }
}

#[derive(Clone, Debug)]
pub struct NoteDoc {
    pub path: PathBuf,
    pub title: String,
    pub content: String,
    pub format: TextFormat,
    pub board: Board,
    pub note_id: Uuid,
}

impl NoteDoc {
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("读取笔记失败: {}", path.display()))?;
        let format = TextFormat::from_path(path);
        let title = note_title(path);

        let mut board = match fs::read_to_string(board_path(path)) {
            Ok(source) => serde_json::from_str::<Board>(&source)
                .with_context(|| format!("解析白板数据失败: {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Board::with_note(content.clone(), format)
            }
            Err(error) => {
                return Err(error).with_context(|| format!("读取白板数据失败: {}", path.display()));
            }
        };

        repair_corrupted_mind_titles(&mut board.nodes);

        let note_id = board.ensure_note_node(&content, format);

        Ok(Self {
            path: path.to_path_buf(),
            title,
            content,
            format,
            board,
            note_id,
        })
    }

    pub fn create(path: &Path, content: String, format: TextFormat, board: Board) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败: {}", parent.display()))?;
        }
        fs::write(path, &content).with_context(|| format!("写入笔记失败: {}", path.display()))?;

        let mut board = board;
        let note_id = board.ensure_note_node(&content, format);
        let doc = Self {
            path: path.to_path_buf(),
            title: note_title(path),
            content,
            format,
            board,
            note_id,
        };
        doc.save_board()?;
        Ok(doc)
    }

    pub fn save_content(&self) -> Result<()> {
        fs::write(&self.path, &self.content)
            .with_context(|| format!("保存笔记失败: {}", self.path.display()))
    }

    pub fn save_board(&self) -> Result<()> {
        let path = board_path(&self.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败: {}", parent.display()))?;
        }
        let source = serde_json::to_string_pretty(&self.board).context("序列化白板数据失败")?;
        fs::write(&path, source).with_context(|| format!("保存白板数据失败: {}", path.display()))
    }

    pub fn sync_note_content(&mut self) {
        if let Some(node) = self.board.node_mut(self.note_id) {
            node.kind = NodeKind::Note {
                content: self.content.clone(),
                format: self.format,
            };
        }
    }
}

pub fn board_path(note_path: &Path) -> PathBuf {
    let mut path = note_path.as_os_str().to_owned();
    path.push(".board.json");
    PathBuf::from(path)
}

pub fn note_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("未命名笔记")
        .to_string()
}

#[derive(Clone, Debug)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub format: Option<TextFormat>,
}

pub fn data_root() -> PathBuf {
    if let Ok(value) = std::env::var("WHITEBOARD_HOME")
        && !value.trim().is_empty()
    {
        return PathBuf::from(value);
    }

    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("WhiteboardNotes")
}

pub fn ensure_data_root(root: &Path, language: Language) -> Result<()> {
    fs::create_dir_all(root).with_context(|| format!("创建数据目录失败: {}", root.display()))?;

    if contains_note(root) {
        return Ok(());
    }

    let (content, tip, mind_root, mind_markdown, mind_images, file_name) = match language {
        Language::Zh => (
            "# 欢迎使用白板笔记\n\n这是一个**无限白板**笔记工具。\n\n## 快速上手\n\n- 左侧 **新建目录 / 新建笔记** 管理文件\n- 右侧工具栏 **新增文本**、**思维导图**、**粘贴图片**\n- **拖动空白**平移画布，**滚轮**缩放\n- **双击卡片**编辑内容，`Ctrl+V` 粘贴剪贴板图片\n- 文本卡片支持 Markdown 与纯文本两种格式\n\n> 所有内容自动保存在本机的笔记目录中。\n",
            "### 提示\n\n- 双击卡片编辑\n- 右上角缩放画布\n- 选中思维导图节点后可添加子节点",
            "无限白板",
            "Markdown 笔记",
            "图片与思维导图",
            "欢迎使用.md",
        ),
        Language::En => (
            "# Welcome to Whiteboard Notes\n\nThis is an **infinite whiteboard** note-taking tool.\n\n## Getting started\n\n- Create folders and notes from the left sidebar\n- Add **text cards**, **mind maps** and **pasted images** from the toolbar\n- **Drag empty space** to pan and use the **mouse wheel** to move around\n- **Double-click a card** to edit it, or press `Ctrl+V` to paste an image\n- Text cards support both Markdown and plain text\n\n> Everything is saved locally in your notes folder.\n",
            "### Tips\n\n- Double-click a card to edit\n- Use the top-right controls to zoom\n- Select a mind map node to add a child",
            "Infinite board",
            "Markdown notes",
            "Images & mind maps",
            "Welcome.md",
        ),
    };

    let mut board = Board::with_note(content, TextFormat::Markdown);
    if let Some(note) = board.note_node_mut() {
        note.x = 96.0;
        note.y = 96.0;
        note.w = 480.0;
        note.h = 380.0;
    }

    board
        .nodes
        .push(BoardNode::text(104.0, 520.0, tip, TextFormat::Markdown));

    let root_node = BoardNode::mind(700.0, 220.0, mind_root, None);
    let root_id = root_node.id;
    board.nodes.push(root_node);
    board
        .nodes
        .push(BoardNode::mind(1000.0, 140.0, mind_markdown, Some(root_id)));
    board
        .nodes
        .push(BoardNode::mind(1000.0, 280.0, mind_images, Some(root_id)));

    let path = root.join(file_name);
    NoteDoc::create(&path, content.to_string(), TextFormat::Markdown, board)?;
    Ok(())
}

fn contains_note(dir: &Path) -> bool {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return false;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() {
            if contains_note(&path) {
                return true;
            }
        } else if matches!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .as_deref(),
            Some("md") | Some("txt")
        ) {
            return true;
        }
    }

    false
}

pub fn scan_tree(root: &Path, expanded: &std::collections::HashSet<PathBuf>) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    scan_dir(root, 0, expanded, &mut entries);
    entries
}

fn scan_dir(
    dir: &Path,
    depth: usize,
    expanded: &std::collections::HashSet<PathBuf>,
    entries: &mut Vec<TreeEntry>,
) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    let mut children = read_dir
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    children.sort_by(|a, b| {
        let a_dir = a.is_dir();
        let b_dir = b.is_dir();
        b_dir.cmp(&a_dir).then_with(|| {
            a.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
                .cmp(
                    &b.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase(),
                )
        })
    });

    for path in children {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if name.starts_with('.') || name == "target" {
            continue;
        }

        if path.is_dir() {
            entries.push(TreeEntry {
                path: path.clone(),
                name,
                is_dir: true,
                depth,
                format: None,
            });

            if expanded.contains(&path) {
                scan_dir(&path, depth + 1, expanded, entries);
            }
            continue;
        }

        let format = match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
        {
            Some(extension) if extension == "md" => TextFormat::Markdown,
            Some(extension) if extension == "txt" => TextFormat::Plain,
            _ => continue,
        };

        entries.push(TreeEntry {
            path,
            name,
            is_dir: false,
            depth,
            format: Some(format),
        });
    }
}

pub fn sanitize_name(name: &str, fallback: &str) -> String {
    let value = name
        .chars()
        .map(|character| match character {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect::<String>();

    let value = value.trim().trim_end_matches('.').trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

pub fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let mut candidate = parent.join(name);
    let mut index = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{name} {index}"));
        index += 1;
    }
    candidate
}

pub fn unique_note_path(parent: &Path, name: &str, format: TextFormat) -> PathBuf {
    let extension = format.extension();
    let mut candidate = parent.join(format!("{name}.{extension}"));
    let mut index = 2;
    while candidate.exists() || board_path(&candidate).exists() {
        candidate = parent.join(format!("{name} {index}.{extension}"));
        index += 1;
    }
    candidate
}

pub fn create_folder(parent: &Path, name: &str, language: Language) -> Result<PathBuf> {
    let fallback = language.pick("新建目录", "New folder");
    let path = unique_dir(parent, &sanitize_name(name, fallback));
    fs::create_dir_all(&path).with_context(|| format!("创建目录失败: {}", path.display()))?;
    Ok(path)
}

pub fn create_note(
    parent: &Path,
    name: &str,
    format: TextFormat,
    language: Language,
) -> Result<NoteDoc> {
    let fallback = language.pick("新建笔记", "New note");
    let name = sanitize_name(name, fallback);
    let path = unique_note_path(parent, &name, format);
    let content = match (language, format) {
        (Language::Zh, TextFormat::Markdown) => format!(
            "# {name}\n\n开始记录你的想法……\n\n- 双击白板上的笔记卡片即可编辑\n- 支持 Markdown 语法\n"
        ),
        (Language::Zh, TextFormat::Plain) => {
            format!("{name}\n\n开始记录你的想法……\n")
        }
        (Language::En, TextFormat::Markdown) => format!(
            "# {name}\n\nStart writing your ideas...\n\n- Double-click the note card to edit\n- Markdown is supported\n"
        ),
        (Language::En, TextFormat::Plain) => {
            format!("{name}\n\nStart writing your ideas...\n")
        }
    };
    let board = Board::with_note(content.clone(), format);
    NoteDoc::create(&path, content, format, board)
}

pub fn validate_note(path: &Path) -> bool {
    path.is_file()
        && matches!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .as_deref(),
            Some("md") | Some("txt")
        )
}

pub fn read_clipboard_image(root: &Path) -> Result<Option<(PathBuf, f32, f32)>> {
    let mut clipboard = arboard::Clipboard::new().context("无法访问系统剪贴板")?;
    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(arboard::Error::ContentNotAvailable) => return Ok(None),
        Err(arboard::Error::ClipboardNotSupported) => {
            return Err(anyhow!("当前系统不支持剪贴板图片"));
        }
        Err(error) => return Err(error).context("读取剪贴板图片失败"),
    };

    let width = image.width as u32;
    let height = image.height as u32;
    if width == 0 || height == 0 {
        return Ok(None);
    }

    let Some(buffer) = image::RgbaImage::from_raw(width, height, image.bytes.into_owned()) else {
        return Err(anyhow!("剪贴板图片数据无效"));
    };

    let assets = root.join(".assets");
    fs::create_dir_all(&assets)
        .with_context(|| format!("创建资源目录失败: {}", assets.display()))?;

    let file_name = format!("image-{}.png", Uuid::new_v4());
    let path = assets.join(file_name);
    buffer
        .save(&path)
        .with_context(|| format!("保存图片失败: {}", path.display()))?;

    Ok(Some((path, width as f32, height as f32)))
}

pub fn save_generated_image(root: &Path, bytes: &[u8]) -> Result<(PathBuf, f32, f32)> {
    use image::GenericImageView as _;

    let image = image::load_from_memory(bytes).context("generated image could not be decoded")?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(anyhow!("generated image has invalid dimensions"));
    }

    let assets = root.join(".assets");
    fs::create_dir_all(&assets)
        .with_context(|| format!("failed to create assets folder: {}", assets.display()))?;
    let path = assets.join(format!("ai-{}.png", Uuid::new_v4()));
    image
        .save(&path)
        .with_context(|| format!("failed to save generated image: {}", path.display()))?;

    Ok((path, width as f32, height as f32))
}

pub fn save_generated_svg(root: &Path, svg: &str) -> Result<(PathBuf, f32, f32)> {
    let svg = svg.trim();
    if !svg.to_ascii_lowercase().starts_with("<svg") {
        return Err(anyhow!("generated SVG is missing the <svg> root element"));
    }
    let (width, height) = svg_dimensions(svg).unwrap_or((800.0, 600.0));

    let assets = root.join(".assets");
    fs::create_dir_all(&assets)
        .with_context(|| format!("failed to create assets folder: {}", assets.display()))?;
    let path = assets.join(format!("ai-{}.svg", Uuid::new_v4()));
    fs::write(&path, svg.as_bytes())
        .with_context(|| format!("failed to save generated SVG: {}", path.display()))?;

    Ok((path, width, height))
}

fn svg_dimensions(svg: &str) -> Option<(f32, f32)> {
    let width = svg_attribute(svg, "width").and_then(parse_svg_length);
    let height = svg_attribute(svg, "height").and_then(parse_svg_length);
    if let (Some(width), Some(height)) = (width, height) {
        return Some((width, height));
    }

    let view_box = svg_attribute(svg, "viewBox").or_else(|| svg_attribute(svg, "viewbox"))?;
    let values = view_box
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.len() < 4 {
        return None;
    }
    let width = parse_svg_length(values[2])?;
    let height = parse_svg_length(values[3])?;
    Some((width, height))
}

fn svg_attribute<'a>(svg: &'a str, name: &str) -> Option<&'a str> {
    for quote in ['"', '\''] {
        let needle = format!("{name}={quote}");
        if let Some(start) = svg.find(&needle) {
            let rest = &svg[start + needle.len()..];
            if let Some(end) = rest.find(quote) {
                return Some(&rest[..end]);
            }
        }
    }
    None
}

fn parse_svg_length(value: &str) -> Option<f32> {
    let number = value
        .trim()
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+'))
        .collect::<String>();
    number
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

pub fn read_clipboard_text() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok().filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_mind_titles_keep_the_minimum_height() {
        let node = BoardNode::mind(0.0, 0.0, "Short topic", None);
        assert_eq!(node.h, 58.0);
    }

    #[test]
    fn long_mind_titles_grow_to_fit_wrapped_lines() {
        let node = BoardNode::mind(
            0.0,
            0.0,
            "这是一个非常长的思维导图主题内容，需要在节点内部自动换行并调整高度以适应多行文字显示",
            None,
        );
        assert!(node.h > 58.0);
    }

    #[test]
    fn old_boards_without_connections_still_load() {
        let board: Board = serde_json::from_str("{}").expect("empty board should deserialize");
        assert!(board.connections.is_empty());
    }

    #[test]
    fn node_connections_round_trip_through_json() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        let mut board = Board::default();
        board.connections.push(NodeConnection::new(from, to));

        let source = serde_json::to_string(&board).expect("board should serialize");
        let restored: Board = serde_json::from_str(&source).expect("board should deserialize");
        assert_eq!(restored.connections.len(), 1);
        assert_eq!(restored.connections[0].from, from);
        assert_eq!(restored.connections[0].to, to);
    }

    #[test]
    fn corrupted_mind_titles_are_repaired() {
        let root = BoardNode::mind(0.0, 0.0, "\u{fffd}\u{fffd}\u{fffd}\u{fffd}", None);
        let root_id = root.id;
        let child = BoardNode::mind(300.0, 0.0, "\u{fffd}\u{fffd}\u{fffd}", Some(root_id));
        let mut nodes = vec![root, child];
        assert!(repair_corrupted_mind_titles(&mut nodes));
        let titles = nodes
            .iter()
            .map(|node| match &node.kind {
                NodeKind::Mind { title, .. } => title.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["中心主题", "子主题"]);
        assert!(!repair_corrupted_mind_titles(&mut nodes));
    }

    #[test]
    fn locked_flag_round_trips_through_json() {
        let mut node = BoardNode::text(0.0, 0.0, "locked", TextFormat::Plain);
        node.locked = true;
        let source = serde_json::to_string(&node).expect("node should serialize");
        let restored: BoardNode = serde_json::from_str(&source).expect("node should deserialize");
        assert!(restored.locked);
    }

    #[test]
    fn deleted_note_card_is_not_recreated_on_load() {
        let mut board = Board::with_note("content", TextFormat::Markdown);
        let note_id = board.note_node_mut().expect("note node").id;
        board.nodes.retain(|node| node.id != note_id);
        board.note_card_deleted = true;

        let restored_id = board.ensure_note_node("content", TextFormat::Markdown);
        assert_eq!(restored_id, Uuid::nil());
        assert!(board.note_node_mut().is_none());
    }

    #[test]
    fn legacy_connection_labels_are_ignored() {
        let source = r#"{
            "connections": [{
                "id": "11111111-1111-4111-8111-111111111111",
                "from": "22222222-2222-4222-8222-222222222222",
                "to": "33333333-3333-4333-8333-333333333333",
                "label": "supports"
            }]
        }"#;
        let board: Board = serde_json::from_str(source).expect("legacy board should deserialize");
        assert_eq!(board.connections.len(), 1);
    }
}
