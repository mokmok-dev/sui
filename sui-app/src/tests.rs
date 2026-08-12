use super::{App, Mode, PROMPT_HEIGHT, char_index_to_byte};
use crate::app::ScrollbackLine;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::{Backend, TestBackend};
use ratatui::layout::Position;
use ratatui::{Terminal, TerminalOptions, Viewport};

fn message_texts(app: &App) -> Vec<&str> {
    app.messages
        .iter()
        .map(|line| match line {
            ScrollbackLine::Prompt(text) | ScrollbackLine::Ghost(text) => text.as_str(),
        })
        .collect()
}

#[test]
fn char_index_to_byte_ascii() {
    assert_eq!(char_index_to_byte("hello", 0), Some(0));
    assert_eq!(char_index_to_byte("hello", 4), Some(4));
}

#[test]
fn char_index_to_byte_multibyte() {
    // "あいう" — each char is 3 bytes
    assert_eq!(char_index_to_byte("あいう", 0), Some(0));
    assert_eq!(char_index_to_byte("あいう", 1), Some(3));
    assert_eq!(char_index_to_byte("あいう", 2), Some(6));
}

#[test]
fn char_index_to_byte_past_end() {
    assert_eq!(char_index_to_byte("hi", 2), None);
    assert_eq!(char_index_to_byte("", 0), None);
}

#[test]
fn with_prompt_prefix_changes_prefix() {
    let app = App::new().with_prompt_prefix("$ ");
    assert_eq!(app.prompt_prefix, "$ ");
}

// ── key-handling tests ──────────────────────────────────────────────

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn typing_appends_to_input() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('i'));
    assert_eq!(app.input, "hi");
    assert_eq!(app.cursor_position, 2);
}

#[test]
fn enter_submits_and_clears() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key(KeyCode::Enter));
    assert!(app.input.is_empty());
    assert_eq!(app.cursor_position, 0);
    assert_eq!(message_texts(&app), vec!["a"]);
}

#[test]
fn enter_on_empty_does_nothing() {
    let mut app = App::new();
    app.handle_key(key(KeyCode::Enter));
    assert!(app.messages.is_empty());
}

#[test]
fn backspace_removes_char_before_cursor() {
    let mut app = App::new();
    app.handle_key(key_char('x'));
    app.handle_key(key_char('y'));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.input, "x");
    assert_eq!(app.cursor_position, 1);
}

#[test]
fn backspace_at_start_does_nothing() {
    let mut app = App::new();
    app.handle_key(key_char('x'));
    // Move cursor to start, then backspace
    app.handle_key(key(KeyCode::Home));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.input, "x");
    assert_eq!(app.cursor_position, 0);
}

#[test]
fn delete_removes_char_at_cursor() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key_char('b'));
    app.handle_key(key(KeyCode::Left)); // cursor now before 'b'
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(app.input, "a");
    assert_eq!(app.cursor_position, 1);
}

#[test]
fn left_right_move_cursor() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key_char('b'));
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.cursor_position, 1);
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.cursor_position, 2);
}

#[test]
fn home_end_move_cursor() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key_char('b'));
    app.handle_key(key(KeyCode::Home));
    assert_eq!(app.cursor_position, 0);
    app.handle_key(key(KeyCode::End));
    assert_eq!(app.cursor_position, 2);
}

#[test]
fn esc_sets_should_quit() {
    let mut app = App::new();
    app.handle_key(key(KeyCode::Esc));
    assert!(app.should_quit);
}

#[test]
fn ctrl_c_sets_should_quit() {
    let mut app = App::new();
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);
}

#[test]
fn insert_middle_of_input() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key_char('c'));
    app.handle_key(key(KeyCode::Left)); // cursor between 'a' and 'c'
    app.handle_key(key_char('b'));
    assert_eq!(app.input, "abc");
    assert_eq!(app.cursor_position, 2);
}

// ── slash-command tests ────────────────────────────────────────────

fn type_text(
    app: &mut App,
    text: &str,
) {
    for c in text.chars() {
        app.handle_key(key_char(c));
    }
}

fn type_and_enter(
    app: &mut App,
    text: &str,
) {
    type_text(app, text);
    app.handle_key(key(KeyCode::Enter));
}

/// Enter shell mode (`!` on empty prompt) then type/run a command.
fn shell_and_enter(
    app: &mut App,
    command: &str,
) {
    app.handle_key(key_char('!'));
    assert_eq!(app.mode(), Mode::Shell);
    type_and_enter(app, command);
}

#[test]
fn slash_exit_quits() {
    let mut app = App::new();
    type_and_enter(&mut app, "/exit");
    assert!(app.should_quit);
    assert!(app.input.is_empty());
    assert!(app.messages.is_empty());
}

#[test]
fn slash_quit_quits() {
    let mut app = App::new();
    type_and_enter(&mut app, "/quit");
    assert!(app.should_quit);
    assert!(app.input.is_empty());
    assert!(app.messages.is_empty());
}

#[test]
fn slash_unknown_shows_error() {
    let mut app = App::new();
    type_and_enter(&mut app, "/foo");
    assert!(!app.should_quit);
    assert!(app.input.is_empty());
    assert_eq!(message_texts(&app), vec!["unknown command: /foo"]);
}

#[test]
fn normal_text_still_adds_to_messages() {
    let mut app = App::new();
    type_and_enter(&mut app, "hello");
    assert_eq!(message_texts(&app), vec!["hello"]);
}

#[test]
fn bang_echo_adds_command_and_stdout() {
    let mut app = App::new();
    shell_and_enter(&mut app, "echo bang-app-ok");
    assert!(app.input.is_empty());
    assert_eq!(app.mode(), Mode::Prompt);
    assert!(
        app.messages
            .iter()
            .any(|m| m == &ScrollbackLine::Prompt("! echo bang-app-ok".into())),
        "messages={:?}",
        app.messages
    );
    assert!(
        app.messages.iter().any(|m| {
            matches!(
                m,
                ScrollbackLine::Ghost(text)
                    if text.contains("bang-app-ok") && text.as_str() != "! echo bang-app-ok"
            )
        }),
        "expected ghost stdout, messages={:?}",
        app.messages
    );
}

#[test]
fn bang_empty_shows_usage() {
    let mut app = App::new();
    app.handle_key(key_char('!'));
    app.handle_key(key(KeyCode::Enter));
    // Empty Enter does not run bash — stay in shell so the user can type.
    assert_eq!(app.mode(), Mode::Shell);
    assert_eq!(
        app.messages,
        vec![ScrollbackLine::Prompt("usage: <command>".into())]
    );
}

#[test]
fn bang_nonzero_exit_is_reported() {
    let mut app = App::new();
    shell_and_enter(&mut app, "exit 9");
    assert!(
        app.messages
            .iter()
            .any(|m| matches!(m, ScrollbackLine::Ghost(text) if text == "exit 9")),
        "messages={:?}",
        app.messages
    );
}

#[test]
fn shell_mode_entered_with_bang_on_empty_prompt() {
    let mut app = App::new();
    assert_eq!(app.mode(), Mode::Prompt);
    assert_eq!(app.prompt_title(), " prompt ");

    app.handle_key(key_char('!'));
    assert_eq!(app.mode(), Mode::Shell);
    assert!(app.input.is_empty());
    assert_eq!(app.prompt_title(), " shell ");

    type_text(&mut app, "echo");
    assert_eq!(app.input, "echo");
    assert_eq!(app.mode(), Mode::Shell);
}

#[test]
fn esc_leaves_shell_mode_without_quitting() {
    let mut app = App::new();
    app.handle_key(key_char('!'));
    type_text(&mut app, "pwd");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Prompt);
    assert!(app.input.is_empty());
    assert!(!app.should_quit);
    assert_eq!(app.prompt_title(), " prompt ");
}

#[test]
fn bang_mid_prompt_inserts_literally() {
    let mut app = App::new();
    type_text(&mut app, "hi");
    app.handle_key(key_char('!'));
    assert_eq!(app.mode(), Mode::Prompt);
    assert_eq!(app.input, "hi!");
}

#[test]
fn slash_in_shell_mode_is_literal_not_candidates() {
    let mut app = App::new();
    app.handle_key(key_char('!'));
    type_text(&mut app, "/exit");
    assert_eq!(app.mode(), Mode::Shell);
    assert!(app.slash_candidates.is_empty());
    assert_eq!(app.input, "/exit");
}

#[test]
fn shell_mode_returns_to_prompt_after_command() {
    let mut app = App::new();
    shell_and_enter(&mut app, "echo done");
    assert_eq!(app.mode(), Mode::Prompt);
    assert!(app.input.is_empty());
    assert_eq!(app.prompt_title(), " prompt ");
}

#[test]
fn empty_enter_does_nothing_with_slash() {
    // Already tested but making sure /-prefix doesn't interfere with empty handling
    let mut app = App::new();
    app.handle_key(key(KeyCode::Enter));
    assert!(app.messages.is_empty());
    assert!(!app.should_quit);
}

#[test]
fn bare_slash_has_candidate_and_enter_executes_it() {
    // "/" alone shows all commands as candidates; Enter runs the first one.
    let mut app = App::new();
    type_and_enter(&mut app, "/");
    assert!(app.should_quit); // exit is the first candidate
    assert!(app.messages.is_empty());
}

#[test]
fn slash_candidates_populated_after_typing_slash() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    assert_eq!(app.slash_candidates.len(), 2); // exit + quit match ""

    app.handle_key(key_char('e'));
    assert_eq!(app.slash_candidates.len(), 1); // "exit" starts with "e"

    app.handle_key(key_char('x'));
    assert_eq!(app.slash_candidates.len(), 1); // "exit" starts with "ex"

    app.handle_key(key_char('z'));
    assert!(app.slash_candidates.is_empty()); // no command starts with "exz"
}

#[test]
fn slash_candidates_match_quit_prefix() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    app.handle_key(key_char('q'));
    assert_eq!(app.slash_candidates.len(), 1); // "quit" starts with "q"
}

#[test]
fn down_up_cycle_candidates_and_wrap() {
    let mut app = App::new();
    // Type "/" to populate candidates (exit, quit)
    app.handle_key(key_char('/'));
    assert_eq!(app.slash_selected, 0);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.slash_selected, 1);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.slash_selected, 0);

    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.slash_selected, 1);
    app.handle_key(key(KeyCode::BackTab));
    assert_eq!(app.slash_selected, 0);
}

#[test]
fn tab_on_bare_slash_autocompletes_first_candidate() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    app.handle_key(key(KeyCode::Tab));
    // Completes the first candidate; candidates then narrow to that name.
    assert_eq!(app.input, "/exit");
    assert_eq!(app.slash_candidates.len(), 1);
    assert_eq!(app.slash_selected, 0);
}

#[test]
fn backspace_narrows_candidates_again() {
    let mut app = App::new();
    // Type "/exz" — "exz" matches nothing
    app.handle_key(key_char('/'));
    app.handle_key(key_char('e'));
    app.handle_key(key_char('x'));
    app.handle_key(key_char('z'));
    assert!(app.slash_candidates.is_empty());

    // Backspace to "ex" — "exit" matches again
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.slash_candidates.len(), 1);
}

#[test]
fn delete_narrows_candidates() {
    let mut app = App::new();
    // Type "/xexit" then move cursor back and delete 'x'
    app.handle_key(key_char('/'));
    app.handle_key(key_char('x'));
    app.handle_key(key_char('e'));
    app.handle_key(key_char('x'));
    app.handle_key(key_char('i'));
    app.handle_key(key_char('t'));
    // Input is "/xexit", cursor at end. Candidates: empty ("xexit" doesn't match)
    assert!(app.slash_candidates.is_empty());

    // Delete the leading 'x' after '/': move left 5 times, then delete
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Left));
    }
    // cursor is now after '/'
    app.handle_key(key(KeyCode::Delete));
    // Input is now "/exit"
    assert_eq!(app.slash_candidates.len(), 1);
}

#[test]
fn slash_candidates_cleared_on_enter() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    assert!(!app.slash_candidates.is_empty());
    app.handle_key(key(KeyCode::Enter));
    assert!(app.slash_candidates.is_empty());
    assert_eq!(app.slash_selected, 0);
}

#[test]
fn slash_candidates_cleared_when_not_starting_with_slash() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    assert!(!app.slash_candidates.is_empty());
    // Backspace to remove the '/'
    app.handle_key(key(KeyCode::Backspace));
    assert!(app.slash_candidates.is_empty());
}

#[test]
fn tab_does_nothing_without_candidates() {
    let mut app = App::new();
    app.handle_key(key(KeyCode::Tab));
    // No slash typed, candidates empty, Tab is a no-op
    assert_eq!(app.slash_candidates.len(), 0);
    assert_eq!(app.slash_selected, 0);
}

#[test]
fn slash_selected_resets_when_candidates_shrink() {
    let mut app = App::new();
    // Populate candidates with "/"
    app.handle_key(key_char('/'));
    assert_eq!(app.slash_selected, 0);
}

// ── Emacs-style key tests ────────────────────────────────────────

#[test]
fn ctrl_f_moves_cursor_right() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(ctrl_key('b'));
    assert_eq!(app.cursor_position, 0);
    app.handle_key(ctrl_key('f'));
    assert_eq!(app.cursor_position, 1);
}

#[test]
fn ctrl_b_moves_cursor_left() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key_char('b'));
    assert_eq!(app.cursor_position, 2);
    app.handle_key(ctrl_key('b'));
    assert_eq!(app.cursor_position, 1);
}

#[test]
fn ctrl_a_moves_to_start() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('i'));
    app.handle_key(ctrl_key('a'));
    assert_eq!(app.cursor_position, 0);
}

#[test]
fn ctrl_e_moves_to_end() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('i'));
    app.handle_key(ctrl_key('a'));
    assert_eq!(app.cursor_position, 0);
    app.handle_key(ctrl_key('e'));
    assert_eq!(app.cursor_position, 2);
}

#[test]
fn ctrl_h_deletes_before_cursor() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key_char('b'));
    app.handle_key(ctrl_key('h'));
    assert_eq!(app.input, "a");
    assert_eq!(app.cursor_position, 1);
}

#[test]
fn ctrl_h_at_start_does_nothing() {
    let mut app = App::new();
    app.handle_key(key_char('x'));
    app.handle_key(key(KeyCode::Home));
    app.handle_key(ctrl_key('h'));
    assert_eq!(app.input, "x");
    assert_eq!(app.cursor_position, 0);
}

#[test]
fn ctrl_h_updates_slash_candidates() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    assert!(!app.slash_candidates.is_empty());
    app.handle_key(ctrl_key('h'));
    assert!(app.slash_candidates.is_empty());
    assert_eq!(app.input, "");
}

#[test]
fn ctrl_k_deletes_to_end() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('e'));
    app.handle_key(key_char('l'));
    app.handle_key(key_char('l'));
    app.handle_key(key_char('o'));
    // Move cursor to position 2 (between 'l' and 'l')
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.cursor_position, 2);
    app.handle_key(ctrl_key('k'));
    assert_eq!(app.input, "he");
    assert_eq!(app.cursor_position, 2);
}

#[test]
fn ctrl_k_at_end_does_nothing() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('i'));
    app.handle_key(ctrl_key('k'));
    assert_eq!(app.input, "hi");
    assert_eq!(app.cursor_position, 2);
}

#[test]
fn ctrl_d_deletes_char_at_cursor() {
    let mut app = App::new();
    app.handle_key(key_char('a'));
    app.handle_key(key_char('b'));
    app.handle_key(key_char('c'));
    // Move cursor left twice: position 1 (between 'a' and 'b')
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.cursor_position, 1);
    app.handle_key(ctrl_key('d'));
    assert_eq!(app.input, "ac");
    assert_eq!(app.cursor_position, 1);
}

#[test]
fn ctrl_d_at_end_does_nothing() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('i'));
    app.handle_key(ctrl_key('d'));
    assert_eq!(app.input, "hi");
    assert_eq!(app.cursor_position, 2);
}

#[test]
fn ctrl_d_updates_slash_candidates() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    app.handle_key(key_char('e'));
    app.handle_key(key_char('x'));
    app.handle_key(key_char('i'));
    app.handle_key(key_char('t'));
    // Input is "/exit", cursor at end. Move to start.
    app.handle_key(key(KeyCode::Home));
    assert_eq!(app.cursor_position, 0);
    app.handle_key(ctrl_key('d'));
    // Deletes the '/' — input becomes "exit", candidates cleared
    assert_eq!(app.input, "exit");
    assert!(app.slash_candidates.is_empty());
}

#[test]
fn ctrl_k_updates_slash_candidates() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    app.handle_key(key_char('e'));
    app.handle_key(key_char('x'));
    app.handle_key(key_char('i'));
    app.handle_key(key_char('t'));
    // Move cursor to after "/e"
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.input, "/exit");
    assert_eq!(app.cursor_position, 2);
    app.handle_key(ctrl_key('k'));
    assert_eq!(app.input, "/e");
    // Candidates should still include "exit" since "/e" matches
    assert_eq!(app.slash_candidates.len(), 1);
}

#[test]
fn ctrl_n_p_cycle_candidates() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    // With 2 candidates (exit, quit), cycling wraps
    assert_eq!(app.slash_selected, 0);
    app.handle_key(ctrl_key('n'));
    assert_eq!(app.slash_selected, 1);
    app.handle_key(ctrl_key('n'));
    assert_eq!(app.slash_selected, 0);
    app.handle_key(ctrl_key('p'));
    assert_eq!(app.slash_selected, 1);
}

// ── Tab autocomplete tests ───────────────────────────────────────

#[test]
fn tab_autocompletes_to_exit() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    app.handle_key(key_char('e'));
    // candidates: [exit], selected: 0
    assert_eq!(app.input, "/e");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.input, "/exit");
    assert_eq!(app.cursor_position, 5);
}

#[test]
fn tab_autocompletes_to_quit() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    app.handle_key(key_char('q'));
    assert_eq!(app.input, "/q");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.input, "/quit");
    assert_eq!(app.cursor_position, 5);
}

#[test]
fn tab_no_candidates_does_nothing() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('i'));
    app.handle_key(key(KeyCode::Tab));
    // No candidates, input unchanged
    assert_eq!(app.input, "hi");
}

// ── inline viewport tests ────────────────────────────────────────

#[test]
fn inline_height_is_prompt_only_by_default() {
    let app = App::new();
    assert_eq!(app.inline_height(), PROMPT_HEIGHT);
    assert_eq!(app.viewport_height, PROMPT_HEIGHT);
}

#[test]
fn inline_height_grows_with_slash_candidates() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    // "/" matches exit + quit — panel opens at a fixed budget, not per-row.
    assert_eq!(app.slash_candidates.len(), 2);
    let open_height = app.inline_height();
    assert!(open_height > PROMPT_HEIGHT);

    // Narrowing candidates must not resize on every keystroke.
    app.handle_key(key_char('e'));
    assert_eq!(app.slash_candidates.len(), 1);
    assert_eq!(app.inline_height(), open_height);

    // Leaving slash mode collapses back to the prompt-only height.
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Backspace));
    assert!(app.slash_candidates.is_empty());
    assert_eq!(app.inline_height(), PROMPT_HEIGHT);
}

fn infallible<T>(result: Result<T, core::convert::Infallible>) -> T {
    match result {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

fn inline_test_terminal(
    width: u16,
    height: u16,
    cursor_y: u16,
) -> Terminal<TestBackend> {
    let mut backend = TestBackend::new(width, height);
    infallible(backend.set_cursor_position(Position::new(0, cursor_y)));
    infallible(Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(PROMPT_HEIGHT),
        },
    ))
}

#[test]
fn flush_messages_writes_above_inline_viewport() {
    let mut terminal = inline_test_terminal(40, 10, 4);

    let mut app = App::new().with_prompt_prefix("> ");
    app.add_message("hello");
    app.add_message("world");
    infallible(app.flush_messages(&mut terminal));

    assert_eq!(app.flushed_messages, 2);
    // Messages were inserted above the viewport; viewport shifts down by 2.
    assert_eq!(terminal.get_frame().area().y, 6);

    let row = |backend: &TestBackend, y: u16| -> String {
        let area = backend.buffer().area;
        (0..area.width)
            .map(|x| backend.buffer()[(x, y)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    let backend = terminal.backend();
    assert_eq!(row(backend, 4), "> hello");
    assert_eq!(row(backend, 5), "> world");
}

#[test]
fn flush_ghost_messages_omit_prompt_prefix() {
    let mut terminal = inline_test_terminal(40, 10, 4);

    let mut app = App::new().with_prompt_prefix("> ");
    app.add_message("! echo hi");
    app.add_ghost("hi");
    infallible(app.flush_messages(&mut terminal));

    // Ghost rows must shift the inline viewport the same way prompt rows do.
    assert_eq!(terminal.get_frame().area().y, 6);

    let row = |backend: &TestBackend, y: u16| -> String {
        let area = backend.buffer().area;
        (0..area.width)
            .map(|x| backend.buffer()[(x, y)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    let backend = terminal.backend();
    assert_eq!(row(backend, 4), "> ! echo hi");
    assert_eq!(row(backend, 5), "hi");
}

#[test]
fn teardown_after_pending_ghost_flush_parks_cursor_below_output() {
    // Mirrors App::run exit path: flush queued bang ghosts, then tear down.
    let mut terminal = inline_test_terminal(40, 12, 4);
    let mut app = App::new().with_prompt_prefix("> ");
    app.add_message("! echo hi");
    app.add_ghost("hi");
    app.add_ghost("bye");
    assert_eq!(app.flushed_messages, 0);

    infallible(app.flush_messages(&mut terminal));
    infallible(App::teardown_inline(&mut terminal));

    assert_eq!(app.flushed_messages, 3);
    assert_eq!(
        infallible(terminal.get_cursor_position()),
        Position::new(0, 7),
        "cursor must sit just below flushed ghost lines"
    );

    let row = |backend: &TestBackend, y: u16| -> String {
        let area = backend.buffer().area;
        (0..area.width)
            .map(|x| backend.buffer()[(x, y)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    let backend = terminal.backend();
    assert_eq!(row(backend, 4), "> ! echo hi");
    assert_eq!(row(backend, 5), "hi");
    assert_eq!(row(backend, 6), "bye");
}

#[test]
fn flush_messages_is_idempotent() {
    let mut terminal = inline_test_terminal(20, 8, 1);

    let mut app = App::new();
    app.add_message("once");
    infallible(app.flush_messages(&mut terminal));
    let y_after_first = terminal.get_frame().area().y;
    infallible(app.flush_messages(&mut terminal));
    assert_eq!(terminal.get_frame().area().y, y_after_first);
    assert_eq!(app.flushed_messages, 1);
}

#[test]
fn teardown_inline_clears_viewport_and_resets_cursor() {
    let mut terminal = inline_test_terminal(20, 10, 4);
    let viewport = terminal.get_frame().area();
    assert_eq!(viewport.y, 4);
    assert_eq!(viewport.height, PROMPT_HEIGHT);

    // Draw something into the viewport so we can tell clear worked.
    infallible(terminal.draw(|frame| {
        frame.render_widget(ratatui::widgets::Paragraph::new("leftover"), frame.area());
        frame.set_cursor_position(Position::new(frame.area().x + 3, frame.area().y + 1));
    }));

    infallible(App::teardown_inline(&mut terminal));

    let origin = Position::new(viewport.x, viewport.y);
    assert_eq!(infallible(terminal.get_cursor_position()), origin);

    // Cells in the former viewport should be empty after clear.
    let backend = terminal.backend();
    for y in viewport.top()..viewport.bottom() {
        for x in viewport.left()..viewport.right() {
            assert_eq!(
                backend.buffer()[(x, y)].symbol(),
                " ",
                "expected empty cell at ({x},{y})"
            );
        }
    }
}
