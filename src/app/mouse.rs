// Modified for rztop on 2026-11-02: ordered mouse dispatch and stable drag gestures.
use super::AppState;
use crate::{
    config::Config,
    domain::resizing::{Axis, Divider, ResizeConfig, SplitTarget},
    draw::controls::*,
    input::Key,
    term::Terminal,
};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

struct Drag {
    divider: Divider,
    origin: usize,
    original: ResizeConfig,
    hints: crate::draw::layout::LayoutHints,
}
#[derive(Default)]
pub(super) struct MouseState {
    drag: Option<Drag>,
}

impl MouseState {
    pub fn reconcile(&mut self, config: &mut Config, state: &mut AppState) {
        if self.drag.as_ref().is_some_and(|drag| {
            drag.hints != state.live.layout_hints(config, &state.view, &state.filter)
        }) {
            self.cancel(config, state);
        }
    }
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn cancel(&mut self, config: &mut Config, state: &mut AppState) {
        if let Some(drag) = self.drag.take() {
            config.resizing = drag.original;
            state.render.dirty.mark_layout();
        }
    }
    pub fn handle(
        &mut self,
        event: MouseEvent,
        config: &mut Config,
        state: &mut AppState,
        terminal: &Terminal,
    ) -> Option<Key> {
        let (x, y) = (usize::from(event.column) + 1, usize::from(event.row) + 1);
        {
            let mut screen = terminal.controls.borrow_mut();
            if screen.pointer != Some((x, y)) {
                screen.pointer = Some((x, y));
                state.render.dirty.mark_layout();
            }
        }
        if let Some(drag) = &self.drag {
            match event.kind {
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    let position = if drag.divider.axis == Axis::Horizontal {
                        y
                    } else {
                        x
                    };
                    let weights = drag
                        .divider
                        .dragged(position as isize - drag.origin as isize);
                    match &drag.divider.target {
                        SplitTarget::Stack(path) => {
                            if config.resizing.layout != config.layout_spec().to_string() {
                                config.resizing.splits.clear();
                            }
                            config.resizing.layout = config.layout_spec().to_string();
                            config.resizing.splits.insert(path.clone(), weights);
                        }
                        SplitTarget::Rankings => config.resizing.set_ranking_weights(&weights),
                    }
                    state.render.dirty.mark_layout();
                    if matches!(event.kind, MouseEventKind::Up(_)) {
                        self.drag = None;
                        terminal.controls.borrow_mut().invalidate();
                    }
                    return None;
                }
                _ => return None,
            }
        }
        let mut screen = terminal.controls.borrow_mut();
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            if let Some(id) = screen.control_at(x, y) {
                screen.invalidate();
                drop(screen);
                return activate(id, state);
            }
            if let Some(divider) = screen.divider_at(x, y).cloned() {
                let origin = if divider.axis == Axis::Horizontal {
                    y
                } else {
                    x
                };
                self.drag = Some(Drag {
                    divider,
                    origin,
                    original: config.resizing.clone(),
                    hints: state.live.layout_hints(config, &state.view, &state.filter),
                });
            }
        } else if event.kind == MouseEventKind::Down(MouseButton::Right)
            && let Some(divider) = screen.divider_at(x, y)
        {
            match &divider.target {
                SplitTarget::Stack(path) => {
                    config.resizing.splits.remove(path);
                }
                SplitTarget::Rankings => {
                    config.resizing.process_weights = [1; 4];
                    config.resizing.io_weight = 0;
                }
            }
            state.render.dirty.mark_layout();
            screen.invalidate();
        }
        None
    }
}

fn activate(id: u32, state: &mut AppState) -> Option<Key> {
    let id = id % MODAL;
    if let crate::overlay::ActiveModal::Options(options) = &mut state.overlay.active {
        if options.is_editing() && id != KEY_ENTER && id != KEY_ESCAPE {
            return None;
        }
        if (OPTION_TAB..OPTION_TAB + 100).contains(&id) {
            options.set_cat(
                (id - OPTION_TAB) as usize,
                crate::overlay::options::render::CATEGORIES.len(),
            );
            state.render.dirty.mark_overlay();
            return None;
        }
        for (base, activate) in [(OPTION_NAME, false), (OPTION_VALUE, true)] {
            if (base..base + 100).contains(&id) {
                options.set_selected((id - base) as usize);
                state.render.dirty.mark_overlay();
                return activate.then_some(Key::Enter);
            }
        }
    }
    Some(match id {
        KEY_LEFT => Key::Left,
        KEY_RIGHT => Key::Right,
        KEY_ENTER => Key::Enter,
        KEY_ESCAPE => Key::Escape,
        KEY_PAGEUP => Key::PageUp,
        KEY_PAGEDOWN => Key::PageDown,
        506 => Key::Up,
        507 => Key::Down,
        32 => Key::Space,
        _ => Key::Char(char::from_u32(id)?),
    })
}

pub(super) fn prepare_screen(state: &AppState, config: &Config, terminal: &Terminal) {
    let mut screen = terminal.controls.borrow_mut();
    screen.modal = state.overlay.active.dims_underlay();
    screen.blocked = matches!(
        state.overlay.active.kind(),
        crate::overlay::OverlayKind::Filter | crate::overlay::OverlayKind::OptionsEdit
    );
    screen.dividers = state
        .render
        .cached_layout()
        .map_or_else(Vec::new, |layout| layout.dividers.clone());
    if config.proc.proc_rankings
        && let Some(dim) = state
            .render
            .cached_layout()
            .and_then(|l| l.dims_for(crate::domain::widget_kind::WidgetKind::Proc))
    {
        let sizes = crate::domain::resizing::allocate(
            dim.height,
            &config.resizing.ranking_weights(),
            &[5; 5],
        );
        for before in 0..4 {
            screen.dividers.push(Divider {
                target: SplitTarget::Rankings,
                axis: Axis::Horizontal,
                position: dim.y + sizes[..=before].iter().sum::<usize>(),
                start: dim.x + 1,
                end: dim.x + dim.width,
                before,
                after: before + 1,
                sizes: sizes.clone(),
                minimums: vec![5; 5],
            });
        }
    }
    screen.valid = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: x - 1,
            row: y - 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }
    fn terminal() -> std::mem::ManuallyDrop<Terminal> {
        std::mem::ManuallyDrop::new(Terminal {
            width: 100,
            height: 50,
            sync_enabled: false,
            controls: std::cell::RefCell::new(Screen::default()),
        })
    }
    fn divider() -> Divider {
        Divider {
            target: SplitTarget::Rankings,
            axis: Axis::Horizontal,
            position: 10,
            start: 1,
            end: 50,
            before: 0,
            after: 1,
            sizes: vec![10; 5],
            minimums: vec![5; 5],
        }
    }
    #[test]
    fn both_strokes_drag_stable_clamp_restore_and_reset() {
        for stroke in [10, 11] {
            let terminal = terminal();
            let mut config = Config::new();
            let mut state = AppState::new(&config);
            let mut mouse = MouseState::default();
            {
                let mut screen = terminal.controls.borrow_mut();
                screen.valid = true;
                screen.dividers = vec![divider()];
            }
            mouse.handle(
                event(MouseEventKind::Down(MouseButton::Left), 20, stroke),
                &mut config,
                &mut state,
                &terminal,
            );
            assert!(mouse.dragging());
            mouse.handle(
                event(MouseEventKind::Drag(MouseButton::Left), 20, 40),
                &mut config,
                &mut state,
                &terminal,
            );
            assert_eq!(config.resizing.ranking_weights(), [15, 5, 10, 10, 10]);
            mouse.handle(
                event(MouseEventKind::Drag(MouseButton::Left), 20, stroke),
                &mut config,
                &mut state,
                &terminal,
            );
            assert_eq!(config.resizing.ranking_weights(), [10; 5]);
            mouse.handle(
                event(MouseEventKind::Up(MouseButton::Left), 20, stroke),
                &mut config,
                &mut state,
                &terminal,
            );
            assert!(!mouse.dragging());
            assert!(!terminal.controls.borrow().valid);
            terminal.controls.borrow_mut().valid = true;
            mouse.handle(
                event(MouseEventKind::Down(MouseButton::Right), 20, stroke),
                &mut config,
                &mut state,
                &terminal,
            );
            assert_eq!(config.resizing.ranking_weights(), [1; 5]);
        }
    }
    #[test]
    fn buttons_win_no_release_action_and_modal_blocks_underlay() {
        let terminal = terminal();
        let mut config = Config::new();
        let mut state = AppState::new(&config);
        let mut mouse = MouseState::default();
        {
            let mut screen = terminal.controls.borrow_mut();
            screen.valid = true;
            screen.dividers = vec![divider()];
            screen.output(&format!("\x1b[10;20H{}", button("filter", 'f' as u32)));
        }
        assert_eq!(
            mouse.handle(
                event(MouseEventKind::Down(MouseButton::Left), 20, 10),
                &mut config,
                &mut state,
                &terminal
            ),
            Some(Key::Char('f'))
        );
        assert!(!mouse.dragging());
        assert_eq!(
            mouse.handle(
                event(MouseEventKind::Up(MouseButton::Left), 20, 10),
                &mut config,
                &mut state,
                &terminal
            ),
            None
        );
        {
            let mut screen = terminal.controls.borrow_mut();
            screen.valid = true;
            screen.modal = true;
        }
        assert_eq!(
            mouse.handle(
                event(MouseEventKind::Down(MouseButton::Left), 20, 10),
                &mut config,
                &mut state,
                &terminal
            ),
            None
        );
        assert!(!mouse.dragging());
    }
    #[test]
    fn cancel_restores_unsaved_proportions() {
        let terminal = terminal();
        let mut config = Config::new();
        let mut state = AppState::new(&config);
        let mut mouse = MouseState::default();
        {
            let mut screen = terminal.controls.borrow_mut();
            screen.valid = true;
            screen.dividers = vec![divider()];
        }
        mouse.handle(
            event(MouseEventKind::Down(MouseButton::Left), 20, 10),
            &mut config,
            &mut state,
            &terminal,
        );
        mouse.handle(
            event(MouseEventKind::Drag(MouseButton::Left), 20, 12),
            &mut config,
            &mut state,
            &terminal,
        );
        mouse.cancel(&mut config, &mut state);
        assert_eq!(config.resizing.ranking_weights(), [1; 5]);
    }
}
