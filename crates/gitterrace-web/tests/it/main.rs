//! One integration binary for the page, API and cache tests, linked once. The golden tests, the
//! memory budget, the hostile workbook and the index child stay apart: the golden runner runs
//! each golden by its own test binary, and the others own a global or a release-only run, or
//! count the workers of their own process.

mod appended_pages;
mod blob_tables_and_archives;
mod chart_tips_carry_totals;
mod charts_have_tables;
mod commit_totals;
mod common;
mod compare_branches;
mod contributors_cap;
mod contributors_metric;
mod conventions;
mod counts_agree;
mod cross_file_fragments;
mod dates_have_tips;
mod empty_selection;
mod far_revision;
mod filter_chips;
mod format_html_count;
mod gitlink_paths;
mod hint_rule_links;
mod languages_empty_period;
mod leads_match_the_numbers;
mod log_graph;
mod markup_owners;
mod merges_option;
mod nav_keeps_branch;
mod newer_cache;
mod one_count_scope;
mod one_statistics_layer;
mod outline_loaded;
mod ownership_marks;
mod page_weight;
mod pages_read_well;
mod path_cells_escape;
mod public_error_bodies;
mod refs_one_page;
mod renamed_ownership;
mod running_cache_rewritten;
mod sha_links;
mod shared_markup_pages;
mod stats_chip_anchor;
mod stats_drill;
mod theme;
mod top_tab_follows_section;
mod unreadable_never_shown;
