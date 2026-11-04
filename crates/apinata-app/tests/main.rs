//! The apinata-app integration suite (headless GPUI).
//!
//! Each file beside this one is a module of this single binary, so the
//! suite links once. A new test file must be declared below, or it never runs.

#[path = "../../apinata-core/tests/git_support/mod.rs"]
mod git_support;
#[path = "changes_helpers/mod.rs"]
mod h;
mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    // Const-initialised and destructor-free, so the allocator touching them never allocates.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

/// Forwards to the system allocator and, inside a [`counting`] window, tallies this thread's
/// allocations and requested bytes, so tests running in parallel never see each other's.
struct Counting;

impl Counting {
    fn tally(bytes: usize) {
        if ARMED.with(Cell::get) {
            ALLOCS.with(|c| c.set(c.get() + 1));
            BYTES.with(|c| c.set(c.get() + bytes as u64));
        }
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Self::tally(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        Self::tally(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// What this thread allocated during a [`counting`] window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocated {
    pub count: u64,
    pub bytes: u64,
}

/// Runs `f` with this thread's allocations counted, and returns its result with the tally.
pub fn counting<R>(f: impl FnOnce() -> R) -> (R, Allocated) {
    ALLOCS.with(|c| c.set(0));
    BYTES.with(|c| c.set(0));
    // Disarms on the way out, so a panic in `f` does not leave later allocations counted.
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            ARMED.with(|a| a.set(false));
        }
    }
    ARMED.with(|a| a.set(true));
    let out = {
        let _disarm = Disarm;
        f()
    };
    let tally = Allocated {
        count: ALLOCS.with(Cell::get),
        bytes: BYTES.with(Cell::get),
    };
    (out, tally)
}

mod actor;
mod actor_context;
mod actor_drafts;
mod actor_env;
mod actor_examples;
mod actor_import;
mod actor_orphans;
mod actor_plan_items;
mod actor_reload;
mod actor_secret_cleanup;
mod actor_secret_values;
mod actor_secrets;
mod actor_stop;
mod actor_trash;
mod actor_tree;
mod actor_update;
mod actor_variables;
mod adaptive_split;
mod add_to_environment;
mod approval_prompt;
mod approval_proxy;
mod approval_text;
mod approvals;
mod approvals_start;
mod assert_value;
mod assertions_move;
mod assertions_section;
mod auth_inherit;
mod auth_secrets;
mod auth_section;
mod autosave;
mod banners;
mod bar_rings;
mod binary_first_rows;
mod binary_view_cap;
mod body_beautify;
mod body_binary;
mod body_binary_format;
mod body_copy;
mod body_edit;
mod body_find_escape;
mod body_format;
mod body_gutter;
mod body_json_marker;
mod body_node_copy;
mod body_rows_keys;
mod body_section;
mod body_tables;
mod body_text;
mod body_typing;
mod body_value_actions;
mod body_view;
mod body_wrap;
mod branch_changes;
mod branch_dialog;
mod branch_picker;
mod branch_shell;
mod breadcrumb;
mod bridge;
mod busy;
mod changes_dialog;
mod changes_dialog_commit;
mod changes_dialog_diff_edges;
mod changes_dialog_gone;
mod changes_dialog_keys;
mod changes_dialog_layout;
mod changes_dialog_polish;
mod changes_dialog_qa;
mod changes_dialog_stage;
mod changes_dialog_states;
mod changes_draft;
mod clone_dialog;
mod clone_entry;
mod clone_form;
mod clone_job;
mod closing;
mod closing_drafts;
mod closing_run;
mod collection_label;
mod collection_switcher;
mod commands;
mod compare_dialog;
mod compare_digests;
mod compare_keep;
mod compare_layout;
mod compare_live;
mod compare_pane;
mod compare_reveal;
mod compare_source;
mod compare_view;
mod config_edit_undo;
mod config_editor;
mod config_reload_keeps_typing;
mod config_scripts;
mod confirm;
mod console_dock;
mod console_dock_shell;
mod console_dock_tabs;
mod cookie_jar;
mod cookie_jar_form_focus;
mod cookie_jar_search;
mod cookie_prefs;
mod copy_curl;
mod default_headers_section;
mod demo_offer;
mod derived_credentials;
mod description_keep;
mod dialog_close_x;
mod dialog_escape;
mod dialog_focus;
mod dialog_frame;
mod dialog_guard;
mod dialog_zoom;
mod diff_doc;
mod diff_tint_contrast;
mod disabled_cue;
mod discard_changes;
mod download_model;
mod download_ui;
mod draft_file;
mod drafts_caps;
mod drafts_close;
mod drafts_discard;
mod drafts_keep;
mod drafts_open;
mod drafts_quit;
mod drafts_restore;
mod drafts_save;
mod drafts_shared_key;
mod edit_history;
mod editor_exit_keys;
mod editor_focus_ring;
mod editor_parity;
mod editor_wrap;
mod empty_editors;
mod empty_states;
mod env_duplicates;
mod env_edit_undo;
mod env_editor;
mod env_move;
mod env_orphans;
mod env_rename;
mod env_table;
mod environment_selection;
mod error_block;
mod error_kinds;
mod example_tab;
mod export_collection;
mod export_environment;
mod files;
mod focus_commands;
mod focus_modality;
mod focus_redraw;
mod focus_survives;
mod fonts;
mod form_label_align;
mod generate_code;
mod git_head;
mod git_status_model;
mod git_watch;
mod git_workspace;
mod git_write;
mod global_env_edit;
mod global_env_rename;
mod global_env_selector;
mod global_env_workspace;
mod graphql_complete;
mod graphql_docs;
mod graphql_editor;
mod graphql_highlight;
mod graphql_schema_store;
mod graphql_section;
mod har_export;
mod harness;
mod header_complete;
mod header_hints;
mod history_drafts;
mod history_extras;
mod history_key;
mod history_migration;
mod history_palette;
mod history_palette_search;
mod history_palette_tags;
mod history_pane;
mod history_prefs;
mod history_reach;
mod history_reopen;
mod history_runner;
mod history_search;
mod history_store;
mod history_view;
mod http_version_display;
mod http_version_prefs;
mod icon_button_names;
mod ime_refresh;
mod import_bundles;
mod import_collection;
mod import_curl;
mod import_link;
mod import_link_layout;
mod import_update;
mod import_update_folder;
mod import_update_source;
mod inherited_headers_editor;
mod inherited_headers_section;
mod input_focus_ring;
mod input_padding;
mod invalid_cue;
mod item_chords_scope;
mod kept_digests;
mod kept_digests_store;
mod keyboard_prefs;
mod keyboard_reach;
mod keyboard_record;
mod keymap;
mod keymap_live;
mod keystroke_burst;
mod kv_bulk;
mod kv_description;
mod kv_grid;
mod kv_rows;
mod kv_table;
mod kv_table_all;
mod kv_table_bulk;
mod kv_table_files;
mod kv_table_move;
mod kv_view;
mod large_body;
mod last_response_label;
mod last_response_moves;
mod last_response_send;
mod last_response_store;
mod last_response_stream;
mod last_response_sweep;
mod legacy_convert;
mod legacy_open;
mod live_stream;
mod locked_eyes;
mod locked_hint;
mod log_feed;
mod mask_template;
mod menu_focus;
mod narrow_env;
mod network_cert_focus;
mod network_certificates;
mod network_changed;
mod network_edits;
mod network_hosts;
mod network_prefs;
mod network_secrets;
mod network_secrets_git_env;
mod network_section;
mod network_settings;
mod new_collection_and_env_import;
mod oauth1_editor;
mod oauth2_editor;
mod oauth2_signin;
mod oauth2_tokens;
mod open_external;
mod open_in_place;
mod palette;
mod palette_availability;
mod palette_measure;
mod palette_menu_actions;
mod palette_registry;
mod palette_requests;
mod pane_insets;
mod path_display;
mod persisted;
mod pin_tabs;
mod post_response_editor;
mod preferences_editor_font;
mod preferences_form;
mod preferences_general;
mod preferences_nav;
mod preferences_responses;
mod preferences_scroll;
mod preferences_search;
mod preferences_zoom;
mod prefs_section;
mod presets;
mod profile;
mod prompt_dialog;
mod prompt_runner;
mod prompt_send;
mod proxy_password_async;
mod px_literals;
mod query_mismatch_notice;
mod redo_keys;
mod redraw_scope;
mod reference_workspace;
mod request_bar_buttons;
mod request_edit_keys;
mod request_edit_undo;
mod request_editor;
mod request_editor_sections;
mod request_scroll_cue;
mod request_sections;
mod request_send;
mod request_split;
mod request_timeout;
mod response_address;
mod response_compact;
mod response_console;
mod response_cookies;
mod response_filter;
mod response_find;
mod response_find_keys;
mod response_header;
mod response_header_row;
mod response_header_tools;
mod response_pane;
mod response_redaction;
mod response_secret_redaction;
mod response_states;
mod response_tab_keys;
mod response_tests;
mod response_timing;
mod restore_disk;
mod ring_gap;
mod ring_room;
mod row_move;
mod row_move_edit;
mod runner_export;
mod runner_fail_on_status;
mod runner_layout;
mod runner_model;
mod runner_rerun_failed;
mod runner_retry;
mod runner_saved_options;
mod runner_scrollbar;
mod runner_selection;
mod runner_stream;
mod runner_tab;
mod runtime_vars;
mod runtime_vars_dialog;
mod save_ends_typing;
mod save_example;
mod schema_test_button;
mod schema_test_edit;
mod scratch;
mod scratch_curl;
mod scratch_gaps;
mod scratch_guards;
mod scratch_hidden;
mod scratch_polish;
mod scratch_restart;
mod scratch_save;
mod scratch_send;
mod scratch_shell;
mod scratch_tidy;
mod script_approvals;
mod script_approve;
mod script_complete;
mod script_section;
mod script_snippets;
mod script_switch;
mod script_timeout;
mod script_trust_prompt;
mod script_vars;
mod scroll_follow;
mod scrollbar_thumb;
mod scrollbars;
mod scrub;
mod search_dialog;
mod secret_forms;
mod secret_host_redaction;
mod secret_lock;
mod section_counts;
mod section_snapshots;
mod segment_keys;
mod select_open_fill;
mod send_anywhere;
mod send_model;
mod send_stream;
mod sensitive_runtime_values;
mod session_unknown_kinds;
mod set_variable;
mod settings_docs;
mod settings_tabs;
mod shell;
mod shell_env;
mod short_dialogs;
mod short_room;
mod short_tabs;
mod shortcuts;
mod sidebar;
mod sidebar_autohide;
mod sidebar_batch;
mod sidebar_cap;
mod sidebar_clicks;
mod sidebar_collapse_all;
mod sidebar_drafts;
mod sidebar_drop;
mod sidebar_drop_multi;
mod sidebar_duplicate;
mod sidebar_duplicate_names;
mod sidebar_examples;
mod sidebar_expanded_session;
mod sidebar_filter;
mod sidebar_filter_hint;
mod sidebar_filter_style;
mod sidebar_filter_terms;
mod sidebar_follow;
mod sidebar_git;
mod sidebar_header;
mod sidebar_keys;
mod sidebar_menu;
mod sidebar_model;
mod sidebar_move_selection;
mod sidebar_multi;
mod sidebar_multi_clicks;
mod sidebar_multi_delete;
mod sidebar_multi_drag;
mod sidebar_multi_drag_rules;
mod sidebar_multi_keys;
mod sidebar_multi_menu;
mod sidebar_multi_move;
mod sidebar_multi_render;
mod sidebar_multi_run;
mod sidebar_narrow;
mod sidebar_paste;
mod sidebar_paste_ui;
mod sidebar_pending;
mod sidebar_rename;
mod sidebar_row_style;
mod sidebar_rows;
mod sidebar_run_buttons;
mod sidebar_sort;
mod sidebar_trash;
mod snapshot;
mod snapshot_cost;
mod split_adaptive;
mod split_ratio;
mod stale;
mod start;
mod status_bar;
mod status_row_age;
mod stream_events;
mod stream_events_layout;
mod stream_large_fast;
mod stream_pane;
mod stream_status;
mod stream_stop;
mod strip_overflow;
mod style_icons;
mod tab_area;
mod tab_close_focus;
mod tab_close_right;
mod tab_drag;
mod tab_git;
mod tab_keys;
mod tab_overflow;
mod tab_run_selection;
mod tab_run_selection_extra;
mod tab_strip;
mod tabs;
mod tag_chips;
mod tags_edit;
mod tags_section;
mod theme_contrast;
mod theme_install;
mod theme_json;
mod theme_named;
mod theme_prefs;
mod theme_select;
mod theme_settings;
mod theme_user;
mod timeline_redaction;
mod timing_bar;
mod title_bar;
mod title_bar_menu;
mod title_bar_setting;
mod title_count;
mod title_priority;
mod token_panel;
mod trusted_hosts;
mod typing_keys;
mod ui_checkbox;
mod ui_primitives;
mod update_notice;
mod update_prepare;
mod update_restart;
mod updates;
mod url_paste;
mod url_row;
mod var_body;
mod var_complete;
mod var_complete_fields;
mod var_complete_list;
mod var_fields;
mod var_prompt;
mod var_tokens;
mod var_url;
mod variables_overview;
mod version;
mod watch_dirs;
mod watch_events;
mod watch_requests;
mod watch_rules;
mod watch_set;
mod watch_shell;
mod watch_workspace;
mod window_drop;
mod window_identity;
mod workspace;
mod workspace_tabs;
mod zoom;
mod zoom_keys;
mod zoom_layout;
