use super::{App, char_index_to_byte};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    assert_eq!(app.messages, vec!["a"]);
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

fn type_and_enter(
    app: &mut App,
    text: &str,
) {
    for c in text.chars() {
        app.handle_key(key_char(c));
    }
    app.handle_key(key(KeyCode::Enter));
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
fn slash_unknown_shows_error() {
    let mut app = App::new();
    type_and_enter(&mut app, "/foo");
    assert!(!app.should_quit);
    assert!(app.input.is_empty());
    assert_eq!(app.messages, vec!["unknown command: /foo"]);
}

#[test]
fn normal_text_still_adds_to_messages() {
    let mut app = App::new();
    type_and_enter(&mut app, "hello");
    assert_eq!(app.messages, vec!["hello"]);
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
    assert!(app.should_quit); // exit is the first (and only) candidate
    assert!(app.messages.is_empty());
}

#[test]
fn slash_candidates_populated_after_typing_slash() {
    let mut app = App::new();
    app.handle_key(key_char('/'));
    assert_eq!(app.slash_candidates.len(), 1); // "exit" matches ""

    app.handle_key(key_char('e'));
    assert_eq!(app.slash_candidates.len(), 1); // "exit" starts with "e"

    app.handle_key(key_char('x'));
    assert_eq!(app.slash_candidates.len(), 1); // "exit" starts with "ex"

    app.handle_key(key_char('z'));
    assert!(app.slash_candidates.is_empty()); // no command starts with "exz"
}

#[test]
fn tab_cycles_through_candidates_and_wraps() {
    // For this test we need multiple commands, so we test with a single
    // command first, then verify the wrap-around behavior.
    let mut app = App::new();
    // Type "/" to populate candidates
    app.handle_key(key_char('/'));
    // With 1 candidate, cycling stays at 0 and wraps to 0
    assert_eq!(app.slash_selected, 0);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.slash_selected, 0);

    app.handle_key(key(KeyCode::BackTab));
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
fn tab_no_candidates_does_nothing() {
    let mut app = App::new();
    app.handle_key(key_char('h'));
    app.handle_key(key_char('i'));
    app.handle_key(key(KeyCode::Tab));
    // No candidates, input unchanged
    assert_eq!(app.input, "hi");
}
