use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    widgets::{Block, BorderType, Paragraph, ScrollbarOrientation, StatefulWidgetRef, Widget},
};
use tui_tree_widget::{Scrollbar, Tree, TreeItem, TreeState};

use crate::tui::tabs::{InputMode, SearchInput};

#[derive(Default, PartialEq, Eq)]
pub enum FocusedDocWidget {
    #[default]
    None,
    Search,
    ItemList,
    DocsText,
}

#[derive(Default)]
pub struct Docs {
    pub focused: FocusedDocWidget,
    pub state: TreeState<&'static str>,
    items: Vec<TreeItem<'static, &'static str>>,
    pub docs_query: String,
}

impl StatefulWidgetRef for Docs {
    type State = InputMode;

    fn render_ref(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        use Constraint::{Length, Min, Percentage};

        let split_content = Layout::horizontal([Percentage(40), Percentage(60)]);
        let [left_area, right_area] = split_content.areas(area);

        let vertical = Layout::vertical([Length(3), Min(0)]);
        let [docs_search_area, docs_results_area] = vertical.areas(left_area);

        let im = state.to_owned();
        let docs_search = self.docs_query.to_owned();

        SearchInput.render_ref(docs_search_area, buf, &mut (im, docs_search));

        let items = vec![
            TreeItem::new_leaf("a", "Alfa"),
            TreeItem::new(
                "b",
                "Bravo",
                vec![
                    TreeItem::new_leaf("c", "Charlie"),
                    TreeItem::new(
                        "d",
                        "Delta",
                        vec![
                            TreeItem::new_leaf("e", "Echo"),
                            TreeItem::new_leaf("f", "Foxtrot"),
                        ],
                    )
                    .expect("all item identifiers are unique"),
                    TreeItem::new_leaf("g", "Golf"),
                ],
            )
            .expect("all item identifiers are unique"),
            TreeItem::new_leaf("h", "Hotel"),
            TreeItem::new(
                "i",
                "India",
                vec![
                    TreeItem::new_leaf("j", "Juliett"),
                    TreeItem::new_leaf("k", "Kilo"),
                    TreeItem::new_leaf("l", "Lima"),
                    TreeItem::new_leaf("m", "Mike"),
                    TreeItem::new_leaf("n", "November"),
                ],
            )
            .expect("all item identifiers are unique"),
            TreeItem::new_leaf("o", "Oscar"),
            TreeItem::new(
                "p",
                "Papa",
                vec![
                    TreeItem::new_leaf("q", "Quebec"),
                    TreeItem::new_leaf("r", "Romeo"),
                    TreeItem::new_leaf("s", "Sierra"),
                    TreeItem::new_leaf("t", "Tango"),
                    TreeItem::new_leaf("u", "Uniform"),
                    TreeItem::new(
                        "v",
                        "Victor",
                        vec![
                            TreeItem::new_leaf("w", "Whiskey"),
                            TreeItem::new_leaf("x", "Xray"),
                            TreeItem::new_leaf("y", "Yankee"),
                        ],
                    )
                    .expect("all item identifiers are unique"),
                ],
            )
            .expect("all item identifiers are unique"),
            TreeItem::new_leaf("z", "Zulu"),
            // "add(x Number, y Number) -> Number",
            // "subtract(x Number, y Number) -> Number",
            // "serialize<T>(value: T) -> Result",
        ];

        let list_focused = self.focused == FocusedDocWidget::ItemList;

        Tree::new(&items)
            .expect("all item identifiers are unique")
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .fg(if list_focused {
                        Color::Green
                    } else {
                        Color::default()
                    })
                    .title(" Items ")
                    .title_bottom(format!("{:?}", self.state)),
            )
            .experimental_scrollbar(Some(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .track_symbol(None)
                    .end_symbol(None),
            ))
            .highlight_style(
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ")
            .render(docs_results_area, buf);

        Paragraph::new(
            "add(x Number, y Number) -> Number\n    Adds x and y. Returns sum of x and y.",
        )
        .block(Block::bordered().title(" Docs: add.gin "))
        .render(right_area, buf);

        // if !tree_state.selected().is_empty() {
        //     let selected = tree_state.selected().first().unwrap();
        //     let docs = match *selected {
        //         "module:math/add" => {
        //             "add(x Number, y Number) -> Number\n    Adds x and y. Returns sum of x and y."
        //         }
        //         "module:math/subtract" => {
        //             "subtract(x Number, y Number) -> Number\n    Subtracts y from x. Returns difference."
        //         }
        //         "module:serde/serialize" => {
        //             "serialize<T>(value: T) -> Result\n    Serializes the given value. Returns Result containing serialized bytes."
        //         }
        //         _ => "No documentation available for this item.",
        //     };

        //     Paragraph::new(docs)
        //         .block(
        //             Block::bordered()
        //                 .border_type(BorderType::Rounded)
        //                 .title(" Docs: add.gin "),
        //         )
        //         .render(right_area, buf);
        // }

        // TODO: tree list, toplevel is the module each node inside is function/tag
        // hovering the module rhs shows the full text
        // expanding and hovering items will move rhs to the exact item
        // kinda like a buffer symbol thing in vs or zed
        //
        // but then everything is an item which means we can search for it by name
    }
}
