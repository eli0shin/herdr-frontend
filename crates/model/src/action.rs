//! Everything a person can ask the app to do: one [`Action`] per row of the keybinding table
//! (`docs/research/feature-inventory.md` §22), including the keys inside popups, copy mode and
//! the mouse gestures, whether or not it is implemented yet. [`crate::Model::step`] runs them;
//! one that isn't built yet returns [`crate::Effect::NotYet`].
//!
//! Actions a key or the palette can name have a command name ([`Action::name`],
//! [`Action::parse`]). The canonical names are herdr-frontend's; [`Action::parse`] also takes the
//! names Eli's `keys.json` uses (`navigator-status.tab-next`, `mod.open:picker`, …), so his key
//! file reads as is.

use std::fmt;

/// A direction to move focus, swap or resize in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 4] = [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }

    pub fn parse(word: &str) -> Option<Direction> {
        Direction::ALL.into_iter().find(|d| d.as_str() == word)
    }
}

/// Where a split puts the new pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Split {
    /// Beside the pane: a row.
    Right,
    /// Below the pane: a column.
    Down,
}

impl Split {
    pub fn as_str(self) -> &'static str {
        match self {
            Split::Right => "right",
            Split::Down => "down",
        }
    }
}

/// A popup or dialog the keys or mouse open. Each holds the keyboard while open (§10-§12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Popup {
    /// `cmd-shift-p`.
    Palette,
    /// `cmd-,`, `ctrl-g c`.
    Settings,
    /// `cmd-/`: keys help.
    Help,
    /// `cmd-p`: tabs and agents.
    Picker,
    /// `ctrl-g s`.
    SpacePicker,
    /// `cmd-n`, `ctrl-g ctrl-s`: a new space in a picked folder.
    NewSpace,
    /// `ctrl-g /`: which machines' agents show.
    AgentMachines,
    /// `ctrl-g f`: which agent owns a PR.
    PrFinder,
    /// `ctrl-;`: jump to an Agents row by its label.
    AgentJump,
    /// The Agents header's sort button.
    SortMenu,
}

impl Popup {
    pub const ALL: [Popup; 10] = [
        Popup::Palette,
        Popup::Settings,
        Popup::Help,
        Popup::Picker,
        Popup::SpacePicker,
        Popup::NewSpace,
        Popup::AgentMachines,
        Popup::PrFinder,
        Popup::AgentJump,
        Popup::SortMenu,
    ];

    /// The popup's name in command names (`popup.<name>`).
    pub fn as_str(self) -> &'static str {
        match self {
            Popup::Palette => "palette",
            Popup::Settings => "settings",
            Popup::Help => "help",
            Popup::Picker => "picker",
            Popup::SpacePicker => "space-picker",
            Popup::NewSpace => "new-space",
            Popup::AgentMachines => "agent-machines",
            Popup::PrFinder => "pr-finder",
            Popup::AgentJump => "agent-jump",
            Popup::SortMenu => "sort-menu",
        }
    }

    pub fn parse(word: &str) -> Option<Popup> {
        Popup::ALL.into_iter().find(|p| p.as_str() == word)
    }
}

/// App and window commands, carried out by the app (`Effect::App`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppCommand {
    Quit,
    Hide,
    HideOthers,
    /// `ctrl-alt-shift-escape`: set the keys and layout aside (built in, can't be rebound).
    Reset,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleSidebar,
    /// `cmd-shift-l`: the next layout.
    LayoutSwitch,
    /// `ctrl-g d`: close the window; core terminals keep running.
    CloseWindow,
    /// Double-click on a drag row.
    Maximize,
}

impl AppCommand {
    pub const ALL: [AppCommand; 11] = [
        AppCommand::Quit,
        AppCommand::Hide,
        AppCommand::HideOthers,
        AppCommand::Reset,
        AppCommand::ZoomIn,
        AppCommand::ZoomOut,
        AppCommand::ZoomReset,
        AppCommand::ToggleSidebar,
        AppCommand::LayoutSwitch,
        AppCommand::CloseWindow,
        AppCommand::Maximize,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AppCommand::Quit => "app.quit",
            AppCommand::Hide => "app.hide",
            AppCommand::HideOthers => "app.hide_others",
            AppCommand::Reset => "app.reset",
            AppCommand::ZoomIn => "app.zoom_in",
            AppCommand::ZoomOut => "app.zoom_out",
            AppCommand::ZoomReset => "app.zoom_reset",
            AppCommand::ToggleSidebar => "sidebar.toggle",
            AppCommand::LayoutSwitch => "layout.switch",
            AppCommand::CloseWindow => "window.close",
            AppCommand::Maximize => "window.maximize",
        }
    }
}

/// Commands for the focused terminal (the `terminal` key context), carried out by its view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TerminalAction {
    /// `cmd-f`: the find bar.
    Search,
    /// `cmd-c`: copy the selection. With nothing selected the key goes to the program.
    Copy,
    /// `cmd-v`, `ctrl-v`.
    Paste,
    /// `ctrl-shift-v`.
    PasteImage,
    /// Select the screen and history (palette only).
    SelectAll,
    /// `send:<text>`: bytes to the program, as typed (`alt-left` sends `ESC b`).
    Send(String),
}

/// Keys inside a list popup (space picker, picker, new space, PR finder, machine filter, menus,
/// palette, help) and the prompt and confirm dialogs (§22 "Keys inside Eli's popups").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PopupAction {
    /// ↑, Ctrl+P, K in the machine filter.
    Up,
    /// ↓, Ctrl+N, J in the machine filter.
    Down,
    /// Enter: go, save, confirm, or "only this machine".
    Choose,
    /// Esc (Q in the machine filter): close without choosing.
    Close,
    /// Space in the machine filter: show or hide the machine under the cursor.
    Toggle,
    /// Ctrl+D in the picker: close the selected tab.
    CloseItem,
    /// Page Up: ten rows up, stopping at the first.
    PageUp,
    /// Page Down: ten rows down, stopping at the last.
    PageDown,
    /// The text field changed.
    Query(String),
}

/// Copy mode's keys (§16 and §22 `copy-mode`). The motions run on the terminal's own buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CopyAction {
    /// h, ←.
    Left,
    /// j, ↓.
    Down,
    /// k, ↑.
    Up,
    /// l, →.
    Right,
    /// `0`, Home: the first column.
    LineStart,
    /// `^`: the first text on the row.
    FirstText,
    /// `$`, End: the last text on the row.
    LineEnd,
    /// g: the top of history.
    Top,
    /// G: the bottom of history.
    Bottom,
    /// H, M, L: the top, middle and bottom of the view.
    ViewTop,
    ViewMiddle,
    ViewBottom,
    /// Ctrl+U, Ctrl+D.
    HalfPageUp,
    HalfPageDown,
    /// Ctrl+B / PageUp, Ctrl+F / PageDown.
    PageUp,
    PageDown,
    /// v, Space: select characters (again: drop the selection).
    SelectChars,
    /// V: select lines (again: drop the selection).
    SelectLines,
    /// y, Enter, Cmd+C: copy the selection and leave (no selection: just leave).
    Yank,
    /// q, Ctrl+C, a click outside: leave without copying.
    Quit,
    /// Esc: drop the selection, else leave.
    Escape,
}

impl CopyAction {
    pub const ALL: [CopyAction; 21] = [
        CopyAction::Left,
        CopyAction::Down,
        CopyAction::Up,
        CopyAction::Right,
        CopyAction::LineStart,
        CopyAction::FirstText,
        CopyAction::LineEnd,
        CopyAction::Top,
        CopyAction::Bottom,
        CopyAction::ViewTop,
        CopyAction::ViewMiddle,
        CopyAction::ViewBottom,
        CopyAction::HalfPageUp,
        CopyAction::HalfPageDown,
        CopyAction::PageUp,
        CopyAction::PageDown,
        CopyAction::SelectChars,
        CopyAction::SelectLines,
        CopyAction::Yank,
        CopyAction::Quit,
        CopyAction::Escape,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CopyAction::Left => "left",
            CopyAction::Down => "down",
            CopyAction::Up => "up",
            CopyAction::Right => "right",
            CopyAction::LineStart => "line_start",
            CopyAction::FirstText => "first_text",
            CopyAction::LineEnd => "line_end",
            CopyAction::Top => "top",
            CopyAction::Bottom => "bottom",
            CopyAction::ViewTop => "view_top",
            CopyAction::ViewMiddle => "view_middle",
            CopyAction::ViewBottom => "view_bottom",
            CopyAction::HalfPageUp => "half_page_up",
            CopyAction::HalfPageDown => "half_page_down",
            CopyAction::PageUp => "page_up",
            CopyAction::PageDown => "page_down",
            CopyAction::SelectChars => "select_chars",
            CopyAction::SelectLines => "select_lines",
            CopyAction::Yank => "yank",
            CopyAction::Quit => "quit",
            CopyAction::Escape => "escape",
        }
    }
}

/// Where a dragged pane lands on another (§5 "Drag to rearrange"): the center swaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// The side a drop places the pane on; `None` for the center, which swaps.
    pub fn side(self) -> Option<herdr_shared::Side> {
        use herdr_shared::Side;
        match self {
            Edge::Center => None,
            Edge::Left => Some(Side::Left),
            Edge::Right => Some(Side::Right),
            Edge::Top => Some(Side::Top),
            Edge::Bottom => Some(Side::Bottom),
        }
    }
}

/// One thing a person asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    App(AppCommand),
    Open(Popup),
    /// `ctrl-g [`.
    CopyMode,
    /// `cmd-r`: the review view, or back from it.
    Review,

    // Tabs (§4). Next, previous and numbers count only the shown tabs.
    TabNew,
    TabClose,
    TabNext,
    TabPrev,
    /// 1-9, among the shown tabs.
    TabSelect(u8),
    TabRename,
    /// `ctrl-g o`: back in the tab jump list.
    JumpBack,
    /// `ctrl-g i`.
    JumpForward,

    // Spaces (§3).
    /// `space.new`: a space on this computer (or on `Some(machine)`, from the picker's "new space
    /// on <machine>" rows), in the focused terminal's folder on that machine, else home.
    SpaceNew(Option<String>),
    SpaceNext,
    SpacePrev,
    /// 1-9, in space order.
    SpaceSelect(u8),
    SpaceRename,
    SpaceClose,

    // Agents (§7).
    AgentNext,
    AgentPrev,
    /// 1-10: the Nth Agents row.
    AgentSelect(u8),

    // Panes (§5).
    PaneSplit(Split),
    PaneClose,
    PaneZoom,
    /// `cmd-alt-arrows`.
    PaneGo(Direction),
    /// `ctrl-h/j/k/l`: through Neovim's splits first, then `PaneGo`.
    PaneNavigate(Direction),
    PaneSwap(Direction),
    /// `alt-shift-h/j/k/l`: the nearest divider, 40px.
    PaneResize(Direction),

    /// The focused terminal's own commands.
    Terminal(TerminalAction),
    /// A key inside the open popup.
    Popup(PopupAction),
    /// A key in the agent-jump bar (a label character).
    AgentJumpKey(char),
    /// A key in copy mode.
    Copy(CopyAction),

    // The mouse (§22 "Mouse"), on what it points at. Ids are qualified.
    /// Click a tab pill (or scroll the bar: `TabNext`/`TabPrev`).
    ShowTab(String),
    /// Double-click a tab pill.
    RenameTabOf(String),
    /// A pill's ×.
    CloseTabOf(String),
    /// Press on one pill, release on another: move it to that 0-based place.
    MoveTab {
        tab: String,
        to: usize,
    },
    /// Right-click a tab pill: the tab menu.
    TabMenu(String),
    /// The tab menu's "New tab": a new tab in that space.
    NewTabIn(String),
    /// Click a pane.
    FocusPane(String),
    /// Click an Agents row: focus that terminal's pane.
    FocusAgent(String),
    /// Click a sub-agent row, or Cmd-click a terminal address: peek it.
    Peek(String),
    /// The peek's ✕, or a click outside it.
    ClosePeek,
    /// The peek's "↗ go to" (it has a pane: focus it) or "↗ keep" (give it a pane).
    KeepPeek,
    /// Click an agent's icon or fold chevron. (§22's "fold section" row has no action:
    /// sidebar sections don't collapse, `docs/plan.md`.)
    FoldSubagents(String),
    /// Right-click a pane's terminal (Shift+right-click when the program takes the mouse): the
    /// pane's id.
    TerminalMenu(String),

    // What the popups run on what they list (§10). Ids are qualified.
    /// The space picker's Enter: show a space, on a terminal tab when its selected tab is an
    /// agent's or the review's (`cfg/mods/space-picker/match.js` `terminalTab`).
    ShowSpace(String),
    /// A palette entry: remembered among its recent commands, then run.
    FromPalette(Box<Action>),
    /// The terminal menu's "Rename" for a pane.
    RenamePaneOf(String),
    /// The terminal menu's "Send right-clicks to pane" (`to_program`) and "Use Herdr right-click
    /// menu": the terminal's `herdr.right-click` label.
    RightClicks {
        terminal: String,
        to_program: bool,
    },
    /// The terminal menu's "Send back to <parent>" (`terminal.park`): the pane goes, the terminal
    /// keeps running under its parent.
    Park(String),
    /// The settings dialog: a file of herdr-frontend's home opened in a new tab running
    /// `$EDITOR` (else `nvim`).
    EditFile(String),
    /// Cmd-click a path or URL in a pane.
    OpenLink {
        pane: String,
        link: String,
    },
    /// Drag a pane's top-edge grip onto another pane.
    RearrangePane {
        pane: String,
        target: String,
        edge: Edge,
    },
    /// A divider drag: new sizes for the nodes beside it.
    ResizeSplit {
        sizes: Vec<(String, herdr_shared::Size)>,
    },
    /// A divider drag of the shown tab: the divider after the `index`th shown child of
    /// `container` follows the pointer to `at`, along the split's axis in the tab area.
    DragDivider {
        container: String,
        index: usize,
        at: f32,
    },
}

/// Eli's `keys.json` names that mean a canonical one here.
const ALIASES: &[(&str, &str)] = &[
    ("navigator-status.tab-next", "tab.next"),
    ("navigator-status.tab-prev", "tab.prev"),
    ("navigator-status.jump-list-back", "jump.back"),
    ("navigator-status.jump-list-forward", "jump.forward"),
    ("navigator-status.review", "review"),
    ("navigator-status.left", "pane.navigate:left"),
    ("navigator-status.right", "pane.navigate:right"),
    ("navigator-status.up", "pane.navigate:up"),
    ("navigator-status.down", "pane.navigate:down"),
    ("layout.toggle:sidebar", "sidebar.toggle"),
    ("app.palette", "popup.palette"),
    ("app.settings", "popup.settings"),
    ("app.help", "popup.help"),
    ("mod.open:copy-mode", "copy_mode"),
    ("copy-mode.escape", "copy_mode.escape"),
];

impl Action {
    /// The command name a key file or the palette uses, for actions that need no target.
    /// `None` for mouse actions and popup text.
    pub fn name(&self) -> Option<String> {
        Some(match self {
            Action::App(command) => command.as_str().to_owned(),
            Action::Open(popup) => format!("popup.{}", popup.as_str()),
            Action::CopyMode => "copy_mode".into(),
            Action::Review => "review".into(),
            Action::TabNew => "tab.new".into(),
            Action::TabClose => "tab.close".into(),
            Action::TabNext => "tab.next".into(),
            Action::TabPrev => "tab.prev".into(),
            Action::TabSelect(n) => format!("tab.select:{n}"),
            Action::TabRename => "tab.rename".into(),
            Action::JumpBack => "jump.back".into(),
            Action::JumpForward => "jump.forward".into(),
            Action::SpaceNew(None) => "space.new".into(),
            Action::SpaceNext => "space.next".into(),
            Action::SpacePrev => "space.prev".into(),
            Action::SpaceSelect(n) => format!("space.select:{n}"),
            Action::SpaceRename => "space.rename".into(),
            Action::SpaceClose => "space.close".into(),
            Action::AgentNext => "agent.next".into(),
            Action::AgentPrev => "agent.prev".into(),
            Action::AgentSelect(n) => format!("agent.select:{n}"),
            Action::PaneSplit(split) => format!("pane.split:{}", split.as_str()),
            Action::PaneClose => "pane.close".into(),
            Action::PaneZoom => "pane.zoom".into(),
            Action::PaneGo(d) => format!("pane.go:{}", d.as_str()),
            Action::PaneNavigate(d) => format!("pane.navigate:{}", d.as_str()),
            Action::PaneSwap(d) => format!("pane.swap:{}", d.as_str()),
            Action::PaneResize(d) => format!("pane.resize:{}", d.as_str()),
            Action::Terminal(TerminalAction::Search) => "terminal.search".into(),
            Action::Terminal(TerminalAction::Copy) => "terminal.copy".into(),
            Action::Terminal(TerminalAction::Paste) => "terminal.paste".into(),
            Action::Terminal(TerminalAction::PasteImage) => "terminal.paste_image".into(),
            Action::Terminal(TerminalAction::SelectAll) => "terminal.select_all".into(),
            Action::Terminal(TerminalAction::Send(text)) => format!("send:{text}"),
            Action::Popup(PopupAction::Up) => "popup.up".into(),
            Action::Popup(PopupAction::Down) => "popup.down".into(),
            Action::Popup(PopupAction::Choose) => "popup.choose".into(),
            Action::Popup(PopupAction::Close) => "popup.close".into(),
            Action::Popup(PopupAction::Toggle) => "popup.toggle".into(),
            Action::Popup(PopupAction::CloseItem) => "popup.close_item".into(),
            Action::Popup(PopupAction::PageUp) => "popup.page_up".into(),
            Action::Popup(PopupAction::PageDown) => "popup.page_down".into(),
            Action::Copy(copy) => format!("copy_mode.{}", copy.as_str()),
            Action::Popup(PopupAction::Query(_))
            | Action::SpaceNew(Some(_))
            | Action::ShowSpace(_)
            | Action::FromPalette(_)
            | Action::RenamePaneOf(_)
            | Action::RightClicks { .. }
            | Action::Park(_)
            | Action::EditFile(_)
            | Action::AgentJumpKey(_)
            | Action::ShowTab(_)
            | Action::RenameTabOf(_)
            | Action::CloseTabOf(_)
            | Action::MoveTab { .. }
            | Action::TabMenu(_)
            | Action::NewTabIn(_)
            | Action::FocusPane(_)
            | Action::FocusAgent(_)
            | Action::Peek(_)
            | Action::ClosePeek
            | Action::KeepPeek
            | Action::FoldSubagents(_)
            | Action::TerminalMenu(_)
            | Action::OpenLink { .. }
            | Action::RearrangePane { .. }
            | Action::ResizeSplit { .. }
            | Action::DragDivider { .. } => return None,
        })
    }

    /// Reads a command name: a canonical one ([`Action::name`]) or one of Eli's `keys.json`
    /// names. `send:` takes the text as written (JSON escapes are the key file's business).
    pub fn parse(name: &str) -> Option<Action> {
        if let Some(text) = name.strip_prefix("send:") {
            return Some(Action::Terminal(TerminalAction::Send(text.to_owned())));
        }
        let name = ALIASES
            .iter()
            .find(|(alias, _)| *alias == name)
            .map_or(name, |(_, canonical)| canonical);
        if let Some(command) = AppCommand::ALL.into_iter().find(|c| c.as_str() == name) {
            return Some(Action::App(command));
        }
        if let Some(rest) = name.strip_prefix("copy_mode.") {
            return CopyAction::ALL
                .into_iter()
                .find(|c| c.as_str() == rest)
                .map(Action::Copy);
        }
        // Eli's numbered navigator commands: `navigator-status.tab-3`, `navigator-status.agent-10`.
        if let Some(n) = name.strip_prefix("navigator-status.tab-") {
            return number(n, 9).map(Action::TabSelect);
        }
        if let Some(n) = name.strip_prefix("navigator-status.agent-") {
            return number(n, 10).map(Action::AgentSelect);
        }
        if let Some(popup) = name.strip_prefix("mod.open:") {
            return Popup::parse(popup).map(Action::Open);
        }
        let (head, arg) = match name.split_once(':') {
            Some((head, arg)) => (head, Some(arg)),
            None => (name, None),
        };
        let direction = || arg.and_then(Direction::parse);
        Some(match (head, arg) {
            ("copy_mode", None) => Action::CopyMode,
            ("review", None) => Action::Review,
            ("tab.new", None) => Action::TabNew,
            ("tab.close", None) => Action::TabClose,
            ("tab.next", None) => Action::TabNext,
            ("tab.prev", None) => Action::TabPrev,
            ("tab.select", Some(n)) => Action::TabSelect(number(n, 9)?),
            ("tab.rename", None) => Action::TabRename,
            ("jump.back", None) => Action::JumpBack,
            ("jump.forward", None) => Action::JumpForward,
            ("space.new", None) => Action::SpaceNew(None),
            ("space.next", None) => Action::SpaceNext,
            ("space.prev", None) => Action::SpacePrev,
            ("space.select", Some(n)) => Action::SpaceSelect(number(n, 9)?),
            ("space.rename", None) => Action::SpaceRename,
            ("space.close", None) => Action::SpaceClose,
            ("agent.next", None) => Action::AgentNext,
            ("agent.prev", None) => Action::AgentPrev,
            ("agent.select", Some(n)) => Action::AgentSelect(number(n, 10)?),
            ("pane.split", Some("right")) => Action::PaneSplit(Split::Right),
            ("pane.split", Some("down")) => Action::PaneSplit(Split::Down),
            ("pane.close", None) => Action::PaneClose,
            ("pane.zoom", None) => Action::PaneZoom,
            ("pane.go", Some(_)) => Action::PaneGo(direction()?),
            ("pane.navigate", Some(_)) => Action::PaneNavigate(direction()?),
            ("pane.swap", Some(_)) => Action::PaneSwap(direction()?),
            ("pane.resize", Some(_)) => Action::PaneResize(direction()?),
            ("terminal.search", None) => Action::Terminal(TerminalAction::Search),
            ("terminal.copy", None) => Action::Terminal(TerminalAction::Copy),
            ("terminal.paste", None) => Action::Terminal(TerminalAction::Paste),
            ("terminal.paste_image", None) => Action::Terminal(TerminalAction::PasteImage),
            ("terminal.select_all", None) => Action::Terminal(TerminalAction::SelectAll),
            ("popup.up", None) => Action::Popup(PopupAction::Up),
            ("popup.down", None) => Action::Popup(PopupAction::Down),
            ("popup.choose", None) => Action::Popup(PopupAction::Choose),
            ("popup.close", None) => Action::Popup(PopupAction::Close),
            ("popup.toggle", None) => Action::Popup(PopupAction::Toggle),
            ("popup.close_item", None) => Action::Popup(PopupAction::CloseItem),
            ("popup.page_up", None) => Action::Popup(PopupAction::PageUp),
            ("popup.page_down", None) => Action::Popup(PopupAction::PageDown),
            (popup, None) if popup.starts_with("popup.") => {
                Action::Open(Popup::parse(&popup["popup.".len()..])?)
            }
            _ => return None,
        })
    }
}

/// 1..=`max`.
fn number(text: &str, max: u8) -> Option<u8> {
    text.parse().ok().filter(|n| (1..=max).contains(n))
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(&name),
            None => write!(f, "{self:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action a key can name, once.
    fn named() -> Vec<Action> {
        let mut all: Vec<Action> = AppCommand::ALL.into_iter().map(Action::App).collect();
        all.extend(Popup::ALL.into_iter().map(Action::Open));
        all.extend([
            Action::CopyMode,
            Action::Review,
            Action::TabNew,
            Action::TabClose,
            Action::TabNext,
            Action::TabPrev,
            Action::TabRename,
            Action::JumpBack,
            Action::JumpForward,
            Action::SpaceNext,
            Action::SpaceNew(None),
            Action::SpacePrev,
            Action::SpaceRename,
            Action::SpaceClose,
            Action::AgentNext,
            Action::AgentPrev,
            Action::PaneSplit(Split::Right),
            Action::PaneSplit(Split::Down),
            Action::PaneClose,
            Action::PaneZoom,
            Action::Terminal(TerminalAction::Search),
            Action::Terminal(TerminalAction::Copy),
            Action::Terminal(TerminalAction::Paste),
            Action::Terminal(TerminalAction::PasteImage),
            Action::Terminal(TerminalAction::SelectAll),
            Action::Terminal(TerminalAction::Send("\u{1b}b".into())),
            Action::Popup(PopupAction::Up),
            Action::Popup(PopupAction::Down),
            Action::Popup(PopupAction::Choose),
            Action::Popup(PopupAction::Close),
            Action::Popup(PopupAction::Toggle),
            Action::Popup(PopupAction::CloseItem),
            Action::Popup(PopupAction::PageUp),
            Action::Popup(PopupAction::PageDown),
        ]);
        all.extend((1..=9).map(Action::TabSelect));
        all.extend((1..=9).map(Action::SpaceSelect));
        all.extend((1..=10).map(Action::AgentSelect));
        for d in Direction::ALL {
            all.extend([
                Action::PaneGo(d),
                Action::PaneNavigate(d),
                Action::PaneSwap(d),
                Action::PaneResize(d),
            ]);
        }
        all.extend(CopyAction::ALL.into_iter().map(Action::Copy));
        all
    }

    #[test]
    fn every_named_action_reads_back_from_its_name() {
        let all = named();
        let mut names = std::collections::BTreeSet::new();
        for action in &all {
            let name = action.name().expect("named");
            assert!(names.insert(name.clone()), "{name} is used twice");
            assert_eq!(Action::parse(&name).as_ref(), Some(action), "{name}");
        }
    }

    #[test]
    fn elis_key_file_names_read_as_the_actions_they_run() {
        for (eli, action) in [
            ("navigator-status.tab-next", Action::TabNext),
            ("navigator-status.tab-prev", Action::TabPrev),
            ("navigator-status.tab-1", Action::TabSelect(1)),
            ("navigator-status.tab-9", Action::TabSelect(9)),
            ("navigator-status.agent-1", Action::AgentSelect(1)),
            ("navigator-status.agent-10", Action::AgentSelect(10)),
            ("navigator-status.jump-list-back", Action::JumpBack),
            ("navigator-status.jump-list-forward", Action::JumpForward),
            ("navigator-status.review", Action::Review),
            (
                "navigator-status.left",
                Action::PaneNavigate(Direction::Left),
            ),
            (
                "navigator-status.down",
                Action::PaneNavigate(Direction::Down),
            ),
            ("navigator-status.up", Action::PaneNavigate(Direction::Up)),
            (
                "navigator-status.right",
                Action::PaneNavigate(Direction::Right),
            ),
            (
                "layout.toggle:sidebar",
                Action::App(AppCommand::ToggleSidebar),
            ),
            ("layout.switch", Action::App(AppCommand::LayoutSwitch)),
            ("app.palette", Action::Open(Popup::Palette)),
            ("app.settings", Action::Open(Popup::Settings)),
            ("app.help", Action::Open(Popup::Help)),
            ("mod.open:agent-jump", Action::Open(Popup::AgentJump)),
            ("mod.open:new-space", Action::Open(Popup::NewSpace)),
            ("mod.open:picker", Action::Open(Popup::Picker)),
            ("mod.open:space-picker", Action::Open(Popup::SpacePicker)),
            (
                "mod.open:agent-machines",
                Action::Open(Popup::AgentMachines),
            ),
            ("mod.open:pr-finder", Action::Open(Popup::PrFinder)),
            ("mod.open:copy-mode", Action::CopyMode),
            ("copy-mode.escape", Action::Copy(CopyAction::Escape)),
            ("space.select:3", Action::SpaceSelect(3)),
            ("pane.split:right", Action::PaneSplit(Split::Right)),
            (
                "send:\u{15}",
                Action::Terminal(TerminalAction::Send("\u{15}".into())),
            ),
            ("window.close", Action::App(AppCommand::CloseWindow)),
        ] {
            assert_eq!(Action::parse(eli), Some(action), "{eli}");
        }
    }

    #[test]
    fn names_out_of_range_or_unknown_read_as_nothing() {
        for name in [
            "tab.select:0",
            "tab.select:10",
            "agent.select:11",
            "navigator-status.tab-0",
            "pane.go:sideways",
            "pane.split:left",
            "mod.open:files",
            "popup.nothing",
            "tab.next:1",
            "nonsense",
            "",
        ] {
            assert_eq!(Action::parse(name), None, "{name}");
        }
    }

    #[test]
    fn targeted_actions_have_no_name_and_print_as_debug() {
        let action = Action::FocusPane("p1".into());
        assert_eq!(action.name(), None);
        assert_eq!(action.to_string(), "FocusPane(\"p1\")");
        assert_eq!(Action::TabNew.to_string(), "tab.new");
    }
}
