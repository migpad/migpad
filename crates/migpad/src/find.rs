//! Find and replace: the bar over the text of a window, and the search in the document of its
//! active tab. A search runs once the phrase is entered, not as it is typed. The query is the same
//! in every window — the last one searched for. What a search found is selected, and the bar tells
//! which match it is of how many: a large document is counted a part a frame, and its matches are
//! replaced the same way.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    Action, App, AppContext, Context, Entity, EntityId, FocusHandle, Focusable, Global, HighlightStyle,
    InteractiveElement, IntoElement, Render, SharedString, StyledText, Subscription, Task, UnderlineStyle, WeakEntity,
    Window, div, prelude::*, px, rgb,
};
use migpad_core::document::{Document, EditError};
use migpad_core::history::{EditKind, Selection};
use migpad_core::search::{MatchWalk, Query, QueryError, Search, Template};
use migpad_core::text::TextStore;
use migpad_editor::{ContextMenuEvent, EditorView};
use migpad_ui::{Button, TextField, theme};

use crate::keys;
use crate::modules::find::{
    CloseFind, FindNext, FindPrevious, ReplaceAll, ReplaceOne, ToggleMatchCase, ToggleRegex, ToggleWholeWord,
};
use crate::strings::{Key, fill, number, percent, tr};
use crate::workspace::Workspace;

/// The key context of the bar: its keys act while one of its fields has the focus.
pub const CONTEXT: &str = "FindBar";
/// The key context of the field of the replacement, in the bar: Enter there replaces.
pub const REPLACE_CONTEXT: &str = "ReplaceField";

/// How long a part of counting or collecting replacements may take, in a frame.
const STEP: Duration = Duration::from_millis(8);
/// How often a walk over the matches checks the time.
const CHECK_EVERY: usize = 256;

/// The last query, the same in every window.
#[derive(Default)]
struct LastQuery {
    query: Query,
    replacement: String,
}

impl Global for LastQuery {}

/// An option of the query that the bar toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryOption {
    MatchCase,
    WholeWord,
    Regex,
}

/// Which way a search goes past an end of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wrap {
    /// On from the start, after the end.
    FromTop,
    /// On from the end, before the start.
    FromBottom,
}

/// What the bar tells about the last search.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Told {
    /// The match `index` of `total`, from 1, both `None` while they are counted.
    Found {
        index: Option<usize>,
        total: Option<usize>,
        wrapped: Option<Wrap>,
    },
    NotFound,
    /// The regular expression is not valid: why.
    Invalid(String),
    /// Replacing every match, this share of the text done.
    Replacing(u64),
    Replaced(usize),
    /// The text would grow past what a document holds: nothing was replaced.
    TooLong,
}

/// What a told result is about: it holds while the document has this text, and the selection is
/// the match told of, if one is.
#[derive(Clone, Debug, PartialEq, Eq)]
struct About {
    document: EntityId,
    version: u64,
    selection: Option<Range<usize>>,
}

/// A walk over the matches of a large document a part a frame: counting them, or collecting their
/// replacements. It stops when the text changes.
struct Job {
    document: WeakEntity<Document>,
    version: u64,
    search: Search,
    walk: MatchWalk,
    kind: JobKind,
    /// Does the parts after the first; dropped, it stops.
    _task: Option<Task<()>>,
}

enum JobKind {
    /// Counts the matches, and those before the match `found`, which the bar tells of.
    Count { found: Range<usize>, wrapped: Option<Wrap>, before: usize, total: usize },
    /// Collects the replacements, to apply at once from `selection`.
    Replace { template: Template, replacements: Vec<(Range<usize>, Vec<u8>)>, selection: Selection },
}

/// The bar of a window: the field of the query with its options, and the field of the replacement.
pub struct FindBar {
    workspace: WeakEntity<Workspace>,
    find: Entity<EditorView>,
    replace: Entity<EditorView>,
    /// Whether the row of the replacement shows: Replace… opened the bar.
    replacing: bool,
    told: Option<(Told, Option<About>)>,
    job: Option<Job>,
    _subscriptions: Vec<Subscription>,
}

impl FindBar {
    pub fn new(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| {
            let mut view = EditorView::single_line(window, cx);
            view.set_placeholder(tr(Key::FindPlaceholder), cx);
            view
        });
        let replace = cx.new(|cx| {
            let mut view = EditorView::single_line(window, cx);
            view.set_placeholder(tr(Key::FindReplacePlaceholder), cx);
            view
        });
        // A query typed anew makes what was told of the old one stale.
        let query = find.read(cx).document().clone();
        let edited = cx.observe(&query, |bar: &mut FindBar, _, cx| {
            if bar.told.as_ref().is_some_and(|(told, _)| !matches!(told, Told::Replacing(_))) {
                bar.told = None;
                cx.notify();
            }
        });
        let menus = [&find, &replace].map(|field| context_menu_of(field, &workspace, window, cx));
        let mut subscriptions = vec![edited];
        subscriptions.extend(menus);
        FindBar { workspace, find, replace, replacing: false, told: None, job: None, _subscriptions: subscriptions }
    }

    /// Shows the bar for a search, with the row of the replacement if `replacing`: the field of the
    /// query takes `seed` — the selected text — or the last query, and the focus, all of it selected.
    pub fn open(&mut self, replacing: bool, seed: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.replacing = replacing;
        let last = cx.try_global::<LastQuery>();
        let text = seed.or_else(|| {
            let empty = self.find.read(cx).text(cx).is_empty();
            last.filter(|_| empty).map(|last| last.query.text.clone())
        });
        let replacement = last.map(|last| last.replacement.clone()).unwrap_or_default();
        if let Some(text) = text {
            self.find.update(cx, |field, cx| field.set_text(&text, window, cx));
        }
        if self.replace.read(cx).text(cx).is_empty() && !replacement.is_empty() {
            self.replace.update(cx, |field, cx| field.set_text(&replacement, window, cx));
        }
        self.find.update(cx, |field, cx| field.select_all(window, cx));
        window.focus(&self.find.focus_handle(cx), cx);
        cx.notify();
    }

    /// The field of the bar that has the focus, if one has.
    pub fn focused_field(&self, window: &Window, cx: &App) -> Option<Entity<EditorView>> {
        [&self.find, &self.replace].into_iter().find(|field| field.focus_handle(cx).is_focused(window)).cloned()
    }

    /// The query of the field, and the replacement if its row shows.
    fn query(&self, cx: &App) -> (String, Option<String>) {
        let replacement = self.replacing.then(|| self.replace.read(cx).text(cx));
        (self.find.read(cx).text(cx), replacement)
    }

    fn tell(&mut self, told: Told, about: Option<About>, cx: &mut Context<Self>) {
        // A count going on is of what was told before; replacing all goes on whatever is told.
        if matches!(self.job, Some(Job { kind: JobKind::Count { .. }, .. })) {
            self.job = None;
        }
        self.told = Some((told, about));
        cx.notify();
    }

    /// Whether the row of the replacement shows: the keys of replacing act only then.
    fn is_replacing(&self) -> bool {
        self.replacing
    }

    /// What the bar says now: what it told, if that still holds for the document of the window.
    fn status(&self, cx: &App) -> Option<(String, bool)> {
        let (told, about) = self.told.as_ref()?;
        if let Some(about) = about {
            let workspace = self.workspace.upgrade()?;
            let workspace = workspace.read(cx);
            let document = workspace.document();
            let selection = workspace.editor().read(cx).selection();
            let selected = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
            let holds = document.entity_id() == about.document
                && document.read(cx).version() == about.version
                && about.selection.as_ref().is_none_or(|told| *told == selected);
            if !holds {
                return None;
            }
        }
        Some(told_text(told))
    }

    /// Counts the matches of `search` in `document` for the match `found`: at once if that is
    /// quick, or a part a frame.
    fn count(
        &mut self,
        document: &Entity<Document>,
        search: Search,
        found: Range<usize>,
        wrapped: Option<Wrap>,
        cx: &mut Context<Self>,
    ) {
        // Replacing all goes on: counting would stop it.
        if matches!(self.job, Some(Job { kind: JobKind::Replace { .. }, .. })) {
            return;
        }
        let version = document.read(cx).version();
        let about = Some(About { document: document.entity_id(), version, selection: Some(found.clone()) });
        self.tell(Told::Found { index: None, total: None, wrapped }, about, cx);
        let kind = JobKind::Count { found, wrapped, before: 0, total: 0 };
        self.run(document, version, search, kind, cx);
    }

    /// Replaces every match of `search` in `document` with `template`, as one undo step from
    /// `selection`: the replacements are collected at once if that is quick, or a part a frame,
    /// and then applied at once.
    fn replace_all(
        &mut self,
        document: &Entity<Document>,
        search: Search,
        template: Template,
        selection: Selection,
        cx: &mut Context<Self>,
    ) {
        let version = document.read(cx).version();
        let about = Some(About { document: document.entity_id(), version, selection: None });
        self.tell(Told::Replacing(0), about, cx);
        let kind = JobKind::Replace { template, replacements: Vec::new(), selection };
        self.run(document, version, search, kind, cx);
    }

    fn replaced(&mut self, document: &Entity<Document>, replaced: Result<usize, EditError>, cx: &mut Context<Self>) {
        let doc = document.read(cx);
        let about = Some(About { document: document.entity_id(), version: doc.version(), selection: None });
        let told = match replaced {
            Ok(0) => Told::NotFound,
            Ok(count) => Told::Replaced(count),
            Err(_) => Told::TooLong,
        };
        self.tell(told, about, cx);
    }

    /// Does the first part of a job now, and the rest a part a frame.
    fn run(
        &mut self,
        document: &Entity<Document>,
        version: u64,
        search: Search,
        kind: JobKind,
        cx: &mut Context<Self>,
    ) {
        let walk = MatchWalk::new(0);
        self.job = Some(Job { document: document.downgrade(), version, search, walk, kind, _task: None });
        if self.job_step(cx) {
            return;
        }
        let task = cx.spawn(async move |bar, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(1)).await;
                let done = bar.update(cx, |bar, cx| bar.job_step(cx));
                if done.unwrap_or(true) {
                    break;
                }
            }
        });
        if let Some(job) = &mut self.job {
            job._task = Some(task);
        }
    }

    /// One part of the job: returns whether it is over — done, or stopped by a change of the text.
    fn job_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(job) = &mut self.job else { return true };
        let Some(document) = job.document.upgrade() else {
            self.job = None;
            return true;
        };
        if document.read(cx).version() != job.version {
            if matches!(job.kind, JobKind::Replace { .. }) {
                self.told = None;
                cx.notify();
            }
            self.job = None;
            return true;
        }
        let deadline = Instant::now() + STEP;
        let (done, len) = document.update(cx, |doc, _| {
            let text = doc.contiguous_text();
            let mut done = false;
            for i in 1.. {
                let Some(found) = job.walk.next(&job.search, text) else {
                    done = true;
                    break;
                };
                match &mut job.kind {
                    JobKind::Count { found: told, before, total, .. } => {
                        *before += usize::from(found.start < told.start);
                        *total += 1;
                    }
                    JobKind::Replace { template, replacements, .. } => {
                        let replacement = job.search.replacement(text, found.clone(), template);
                        replacements.push((found, replacement.expect("a match just found")));
                    }
                }
                if i % CHECK_EVERY == 0 && Instant::now() >= deadline {
                    break;
                }
            }
            (done, text.len())
        });
        let Some(job) = self.job.take() else { return true };
        if !done {
            if let JobKind::Replace { .. } = job.kind {
                let share = (job.walk.position().min(len) as u64 * 100).checked_div(len as u64).unwrap_or(100);
                if let Some((told, _)) = &mut self.told {
                    *told = Told::Replacing(share);
                }
                cx.notify();
            }
            self.job = Some(job);
            return false;
        }
        match job.kind {
            JobKind::Count { wrapped, before, total, .. } => {
                // A match found that overlaps one counted is not counted itself.
                let index = (before + 1).min(total.max(1));
                if let Some((told, _)) = &mut self.told {
                    *told = Told::Found { index: Some(index), total: Some(total), wrapped };
                }
                cx.notify();
            }
            JobKind::Replace { replacements, selection, .. } => {
                let replaced = document.update(cx, |doc, cx| {
                    let replaced = doc.apply_replacements(&replacements, selection, Instant::now());
                    if replaced.is_ok_and(|count| count > 0) {
                        cx.notify();
                    }
                    replaced
                });
                self.replaced(&document, replaced, cx);
            }
        }
        true
    }
}

/// Opens the context menu of the field `field` of a bar, as its view asks: the window opens it.
pub fn context_menu_of<T: 'static>(
    field: &Entity<EditorView>,
    workspace: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Subscription {
    let workspace = workspace.clone();
    cx.subscribe_in(field, window, move |_, field, event: &ContextMenuEvent, window, cx| {
        let _ = workspace.update(cx, |workspace, cx| workspace.open_context_menu(field, event, window, cx));
    })
}

/// What the bar says of `told`, and whether it is bad news.
fn told_text(told: &Told) -> (String, bool) {
    match told {
        Told::Found { index: Some(index), total: Some(total), wrapped } => {
            let key = match wrapped {
                None => Key::FindFound,
                Some(Wrap::FromTop) => Key::FindFoundFromTop,
                Some(Wrap::FromBottom) => Key::FindFoundFromBottom,
            };
            (fill(key, &[("index", &number(*index as u64)), ("total", &number(*total as u64))]), false)
        }
        Told::Found { .. } => (tr(Key::FindCounting).to_owned(), false),
        Told::NotFound => (tr(Key::FindNotFound).to_owned(), true),
        Told::Invalid(reason) => (fill(Key::FindInvalid, &[("reason", reason)]), true),
        Told::Replacing(share) => (fill(Key::FindReplacing, &[("percent", &percent(*share))]), false),
        Told::Replaced(count) => (fill(Key::FindReplaced, &[("count", &number(*count as u64))]), false),
        Told::TooLong => (tr(Key::FindTooLong).to_owned(), true),
    }
}

/// The selected text, if it is a line or a part of one: what Find puts in its field.
fn selected_line(workspace: &Workspace, cx: &App) -> Option<String> {
    let selection = workspace.editor().read(cx).selection();
    let range = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
    if range.is_empty() || range.len() > 1024 {
        return None;
    }
    let bytes = workspace.document().read(cx).text().to_vec(range);
    let text = String::from_utf8_lossy(&bytes);
    (!text.contains(['\n', '\r'])).then(|| text.into_owned())
}

/// Find… and Replace…: shows the bar of the window, its field of the query with the selected text.
pub fn open(workspace: &mut Workspace, replacing: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    let seed = selected_line(workspace, cx);
    let bar = workspace.show_find_bar(window, cx);
    bar.update(cx, |bar, cx| bar.open(replacing, seed, window, cx));
}

/// Closes the bar of the window: the focus goes back to the text.
pub fn close(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.hide_find_bar(cx);
    let editor = workspace.editor().focus_handle(cx);
    window.focus(&editor, cx);
}

/// Toggles an option of the query: the next search takes it.
pub fn toggle(option: QueryOption, cx: &mut App) {
    let query = &mut cx.default_global::<LastQuery>().query;
    match option {
        QueryOption::MatchCase => query.match_case = !query.match_case,
        QueryOption::WholeWord => query.whole_word = !query.whole_word,
        QueryOption::Regex => query.regex = !query.regex,
    }
    cx.refresh_windows();
}

fn option_on(option: QueryOption, cx: &App) -> bool {
    let Some(last) = cx.try_global::<LastQuery>() else { return false };
    match option {
        QueryOption::MatchCase => last.query.match_case,
        QueryOption::WholeWord => last.query.whole_word,
        QueryOption::Regex => last.query.regex,
    }
}

/// The search of the last query, or what is wrong with it, told in the bar of the window — shown
/// for that, or for an empty query to type.
fn search(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Option<Search> {
    // The query of the bar becomes the last one, the same in every window.
    if let Some(bar) = workspace.find_bar_shown() {
        let (text, replacement) = bar.read(cx).query(cx);
        let last = cx.default_global::<LastQuery>();
        last.query.text = text;
        if let Some(replacement) = replacement {
            last.replacement = replacement;
        }
    }
    let query = cx.default_global::<LastQuery>().query.clone();
    match Search::new(&query) {
        Ok(search) => Some(search),
        Err(QueryError::Empty) => {
            open(workspace, false, window, cx);
            None
        }
        Err(QueryError::Invalid(message)) => {
            let bar = workspace.show_find_bar(window, cx);
            bar.update(cx, |bar, cx| bar.tell(Told::Invalid(reason(&message)), None, cx));
            None
        }
    }
}

/// The reason in the message of the regex crate: its last line, after "error: ".
fn reason(message: &str) -> String {
    let last = message.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or(message);
    last.trim().strip_prefix("error: ").unwrap_or(last.trim()).to_owned()
}

/// Find Next and Find Previous: selects the next match of the last query after the selection, or
/// the one before it, round the ends of the text.
pub fn find(workspace: &mut Workspace, backwards: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(search) = search(workspace, window, cx) else { return };
    let (document, editor) = (workspace.document().clone(), workspace.editor().clone());
    let selection = editor.read(cx).selection();
    let selected = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
    let found = document.update(cx, |doc, _| {
        let text = doc.contiguous_text();
        if backwards { search.find_prev(text, selected.clone()) } else { search.find_next(text, selected.clone()) }
    });
    let Some(found) = found else {
        let about = About { document: document.entity_id(), version: document.read(cx).version(), selection: None };
        let bar = workspace.show_find_bar(window, cx);
        bar.update(cx, |bar, cx| bar.tell(Told::NotFound, Some(about), cx));
        return;
    };
    let wrapped = match backwards {
        false if found.start < selected.end || found == selected => Some(Wrap::FromTop),
        true if found.start >= selected.start => Some(Wrap::FromBottom),
        _ => None,
    };
    editor.update(cx, |editor, cx| editor.select(Selection { anchor: found.start, head: found.end }, window, cx));
    if let Some(bar) = workspace.find_bar_shown() {
        bar.update(cx, |bar, cx| bar.count(&document, search, found, wrapped, cx));
    }
}

/// Whether the row of the replacement shows: replacing acts only then, its keys too — a hidden
/// replacement would replace unseen.
fn replacing(workspace: &Workspace, cx: &App) -> bool {
    workspace.find_bar_shown().is_some_and(|bar| bar.read(cx).is_replacing())
}

/// Replace: the selected match is replaced, and the next one selected; without a selected match,
/// the next one is found.
pub fn replace_one(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if !replacing(workspace, cx) {
        return;
    }
    let Some(search) = search(workspace, window, cx) else { return };
    let (document, editor) = (workspace.document().clone(), workspace.editor().clone());
    if document.read(cx).is_preview() {
        return;
    }
    let replacement = cx.default_global::<LastQuery>().replacement.clone();
    let selection = editor.read(cx).selection();
    let selected = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
    let bytes = document.update(cx, |doc, _| {
        let template = search.template(&replacement, doc.format.line_ending.as_bytes());
        search.replacement(doc.contiguous_text(), selected.clone(), &template)
    });
    if let Some(bytes) = bytes {
        let after = Selection::caret(selected.start + bytes.len());
        document.update(cx, |doc, cx| {
            if doc.edit(&[(selected.clone(), &bytes)], selection, after, EditKind::Other, Instant::now()).is_ok() {
                cx.notify();
            }
        });
        editor.update(cx, |editor, cx| editor.select(after, window, cx));
    }
    find(workspace, false, window, cx);
}

/// Replace All: every match, as one undo step.
pub fn replace_all(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if !replacing(workspace, cx) {
        return;
    }
    let Some(search) = search(workspace, window, cx) else { return };
    let (document, editor) = (workspace.document().clone(), workspace.editor().clone());
    if document.read(cx).is_preview() {
        return;
    }
    let replacement = cx.default_global::<LastQuery>().replacement.clone();
    let template = search.template(&replacement, document.read(cx).format.line_ending.as_bytes());
    let selection = editor.read(cx).selection();
    let bar = workspace.show_find_bar(window, cx);
    bar.update(cx, |bar, cx| bar.replace_all(&document, search, template, selection, cx));
}

impl Render for FindBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let workspace = self.workspace.clone();
        // The buttons act as their commands do, on the window of the bar.
        let act =
            |id: &'static str, label: SharedString, run: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| {
                let workspace = workspace.clone();
                Button::new(id, label).on_click(move |_, window, cx| {
                    let _ = workspace.update(cx, |workspace, cx| run(workspace, window, cx));
                })
            };
        let (find_focus, replace_focus) = (self.find.focus_handle(cx), self.replace.focus_handle(cx));
        let keys =
            |action: &dyn Action, focus: &FocusHandle| keys::for_action(action, focus, window).map(SharedString::from);
        let toggle =
            |id: &'static str, content: StyledText, name: Key, option: QueryOption, keys: Option<SharedString>| {
                Button::new(id, tr(name))
                    .content(content)
                    .toggled(option_on(option, cx))
                    .tooltip(tr(name), keys)
                    .on_click(move |_, _, cx| toggle(option, cx))
            };
        let underline = HighlightStyle {
            underline: Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
            ..Default::default()
        };
        let status = self.status(cx).map(|(text, bad)| {
            div()
                .flex_none()
                .pl(px(4.))
                .whitespace_nowrap()
                .text_color(rgb(if bad { theme.error_edge } else { theme.text_muted }))
                .child(text)
        });
        let field = |view: &Entity<EditorView>| {
            div().flex_shrink(1.).w(px(300.)).min_w(px(100.)).child(TextField::new(view.clone()))
        };
        let find_row = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(field(&self.find))
            .child(toggle(
                "match-case",
                StyledText::new("Aa"),
                Key::FindMatchCase,
                QueryOption::MatchCase,
                keys(&ToggleMatchCase, &find_focus),
            ))
            .child(toggle(
                "whole-word",
                StyledText::new("ab").with_highlights([(0..2, underline)]),
                Key::FindWholeWord,
                QueryOption::WholeWord,
                keys(&ToggleWholeWord, &find_focus),
            ))
            .child(toggle(
                "regex",
                StyledText::new(".*"),
                Key::FindRegex,
                QueryOption::Regex,
                keys(&ToggleRegex, &find_focus),
            ))
            .child(
                act("find-previous", "↑".into(), |workspace, window, cx| find(workspace, true, window, cx))
                    .name(tr(Key::FindPrevious))
                    .tooltip(tr(Key::FindPrevious), keys(&FindPrevious, &find_focus)),
            )
            .child(
                act("find-next", "↓".into(), |workspace, window, cx| find(workspace, false, window, cx))
                    .name(tr(Key::FindNext))
                    .tooltip(tr(Key::FindNext), keys(&FindNext, &find_focus)),
            )
            .children(status)
            .child(div().flex_1())
            .child(
                act("find-close", "×".into(), close)
                    .name(tr(Key::FindClose))
                    .tooltip(tr(Key::FindClose), keys(&CloseFind, &find_focus)),
            );
        let replace_row = self.replacing.then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().key_context(REPLACE_CONTEXT).child(field(&self.replace)))
                .child(
                    act("replace-one", tr(Key::FindReplaceOne).into(), replace_one)
                        .tooltip(tr(Key::FindReplaceOne), keys(&ReplaceOne, &replace_focus)),
                )
                .child(
                    act("replace-all", tr(Key::FindReplaceAll).into(), replace_all)
                        .tooltip(tr(Key::FindReplaceAll), keys(&ReplaceAll, &replace_focus)),
                )
        });
        div()
            .id("find-bar")
            .key_context(CONTEXT)
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(4.))
            .px(px(8.))
            .py(px(5.))
            .bg(rgb(theme.bar))
            .border_b_1()
            .border_color(rgb(theme.border))
            .child(find_row)
            .children(replace_row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::{LANGUAGE_LOCK, Language, set_language};

    #[test]
    fn the_bar_tells_what_was_found_in_both_languages() {
        let _lock = LANGUAGE_LOCK.lock();
        let found = |index, total, wrapped| told_text(&Told::Found { index: Some(index), total: Some(total), wrapped });
        set_language(Language::Russian);
        assert_eq!(found(3, 12, None), ("3 из 12".to_owned(), false));
        assert_eq!(found(1, 1234, Some(Wrap::FromTop)).0, "1 из 1\u{202f}234, с начала");
        assert_eq!(found(12, 12, Some(Wrap::FromBottom)).0, "12 из 12, с конца");
        assert_eq!(told_text(&Told::NotFound), ("Не найдено".to_owned(), true));
        assert_eq!(told_text(&Told::Replaced(5)).0, "Заменено: 5");
        assert_eq!(told_text(&Told::Replacing(40)).0, "Замена… 40\u{a0}%");
        set_language(Language::English);
        assert_eq!(found(3, 12, None).0, "3 of 12");
        assert_eq!(found(1, 2, Some(Wrap::FromTop)).0, "1 of 2, from the top");
        assert_eq!(told_text(&Told::Found { index: None, total: None, wrapped: None }).0, "Counting…");
        let invalid = told_text(&Told::Invalid("unclosed group".into()));
        assert_eq!(invalid, ("Invalid expression: unclosed group".to_owned(), true));
    }

    #[test]
    fn the_reason_of_a_bad_expression_is_its_last_line() {
        let message = Search::new(&Query { text: "(кот".into(), regex: true, ..Query::default() }).unwrap_err();
        let QueryError::Invalid(message) = message else { panic!("{message:?}") };
        assert_eq!(reason(&message), "unclosed group");
        assert_eq!(reason("something odd"), "something odd");
    }
}
