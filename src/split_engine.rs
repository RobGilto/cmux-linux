use crate::ghostty::ffi;
use gtk4::prelude::*;
use uuid::Uuid;

/// Direction for pane focus navigation (Ctrl+Shift+arrows per D-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Recursive pane layout tree. Each workspace has one root SplitNode.
/// - Leaf: a single terminal pane (GtkGLArea + Ghostty surface)
/// - Split: two child subtrees separated by a GtkPaned divider
///
/// Per SPLIT-06: this is the Bonsplit Rust port — immutable-style tree where
/// split/close operations return a new root.
#[derive(Clone)]
pub enum SplitNode {
    Leaf {
        pane_id: u64,
        gl_area: gtk4::GLArea,
        surface: ffi::ghostty_surface_t,
        /// Stable UUID for session persistence and v2 socket protocol pane identity.
        uuid: Uuid,
        /// Phase 4 NOTF-01: true when this pane has unread bell activity.
        has_attention: bool,
    },
    /// Phase 8: Browser preview pane (agent-browser frame rendering).
    Preview {
        pane_id: u64,
        container: gtk4::Box,
        picture: gtk4::Picture,
        #[allow(dead_code)] // Kept alive for GTK widget tree reference counting
        url_entry: gtk4::Entry,
        uuid: Uuid,
    },
    Split {
        orientation: gtk4::Orientation,
        paned: gtk4::Paned,
        start: Box<SplitNode>,
        end: Box<SplitNode>,
    },
}

impl SplitNode {
    /// Returns the root GTK widget for this node (GLArea for Leaf, Overlay for Preview, Paned for Split).
    pub fn widget(&self) -> gtk4::Widget {
        match self {
            SplitNode::Leaf { gl_area, .. } => gl_area.clone().upcast(),
            SplitNode::Preview { container, .. } => container.clone().upcast(),
            SplitNode::Split { paned, .. } => paned.clone().upcast(),
        }
    }

    /// Find the pane_id of the active (focused) leaf by checking CSS class.
    pub fn find_active_pane_id(&self) -> Option<u64> {
        match self {
            SplitNode::Leaf {
                pane_id, gl_area, ..
            } => {
                if gl_area.has_css_class("active-pane") {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitNode::Preview {
                pane_id, container, ..
            } => {
                if container.has_css_class("active-pane") {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => start
                .find_active_pane_id()
                .or_else(|| end.find_active_pane_id()),
        }
    }

    /// Find the UUID for a pane by pane_id. Returns None if not found.
    pub fn find_uuid_for_pane(&self, target_id: u64) -> Option<String> {
        match self {
            SplitNode::Leaf { pane_id, uuid, .. } | SplitNode::Preview { pane_id, uuid, .. } => {
                if *pane_id == target_id {
                    Some(uuid.to_string())
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => start
                .find_uuid_for_pane(target_id)
                .or_else(|| end.find_uuid_for_pane(target_id)),
        }
    }

    /// Apply the active-pane CSS class to the leaf matching active_pane_id.
    /// Removes the class from all other leaves.
    pub fn update_focus_css(&self, active_pane_id: u64) {
        match self {
            SplitNode::Leaf {
                pane_id, gl_area, ..
            } => {
                if *pane_id == active_pane_id {
                    gl_area.add_css_class("active-pane");
                } else {
                    gl_area.remove_css_class("active-pane");
                }
            }
            SplitNode::Preview {
                pane_id, container, ..
            } => {
                if *pane_id == active_pane_id {
                    container.add_css_class("active-pane");
                } else {
                    container.remove_css_class("active-pane");
                }
            }
            SplitNode::Split { start, end, .. } => {
                start.update_focus_css(active_pane_id);
                end.update_focus_css(active_pane_id);
            }
        }
    }

    /// Find a node by pane_id.
    pub fn find_node(&self, target_id: u64) -> Option<&SplitNode> {
        match self {
            SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => {
                if *pane_id == target_id {
                    Some(self)
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => start
                .find_node(target_id)
                .or_else(|| end.find_node(target_id)),
        }
    }

    /// Collect all leaf pane_ids into a Vec (for cleanup on workspace close).
    pub fn collect_pane_ids(&self, out: &mut Vec<u64>) {
        match self {
            SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => {
                out.push(*pane_id)
            }
            SplitNode::Split { start, end, .. } => {
                start.collect_pane_ids(out);
                end.collect_pane_ids(out);
            }
        }
    }

    /// Collect all surfaces into a Vec (for ghostty_surface_free on workspace close).
    #[allow(dead_code)] // kept: session-restore/debug API surface
    pub fn collect_surfaces(&self, out: &mut Vec<ffi::ghostty_surface_t>) {
        match self {
            SplitNode::Leaf { surface, .. } => out.push(*surface),
            SplitNode::Preview { .. } => {} // No Ghostty surface
            SplitNode::Split { start, end, .. } => {
                start.collect_surfaces(out);
                end.collect_surfaces(out);
            }
        }
    }

    /// Find the Ghostty surface handle for a specific pane by pane_id.
    /// Used by debug.type to send text to a specific pane's surface.
    pub fn find_surface_for_pane(&self, target_id: u64) -> Option<ffi::ghostty_surface_t> {
        match self {
            SplitNode::Leaf {
                pane_id, surface, ..
            } => {
                if *pane_id == target_id {
                    Some(*surface)
                } else {
                    None
                }
            }
            SplitNode::Preview { .. } => None, // No Ghostty surface
            SplitNode::Split { start, end, .. } => start
                .find_surface_for_pane(target_id)
                .or_else(|| end.find_surface_for_pane(target_id)),
        }
    }

    /// Collect (uuid, pane_id, active) for all leaves in this subtree.
    pub fn collect_pane_info(&self, out: &mut Vec<(Uuid, u64, bool)>, active_id: u64) {
        match self {
            SplitNode::Leaf { pane_id, uuid, .. } | SplitNode::Preview { pane_id, uuid, .. } => {
                out.push((*uuid, *pane_id, *pane_id == active_id));
            }
            SplitNode::Split { start, end, .. } => {
                start.collect_pane_info(out, active_id);
                end.collect_pane_info(out, active_id);
            }
        }
    }

    /// Find the ghostty surface handle for the leaf matching target_uuid (UUID string).
    #[allow(dead_code)] // kept: session-restore/debug API surface
    pub fn find_by_uuid(&self, target_uuid: &str) -> Option<ffi::ghostty_surface_t> {
        match self {
            SplitNode::Leaf { uuid, surface, .. } => {
                if uuid.to_string() == target_uuid {
                    Some(*surface)
                } else {
                    None
                }
            }
            SplitNode::Preview { .. } => None, // No Ghostty surface to return
            SplitNode::Split { start, end, .. } => start
                .find_by_uuid(target_uuid)
                .or_else(|| end.find_by_uuid(target_uuid)),
        }
    }

    /// Find the pane_id for the leaf matching target_uuid (UUID string).
    pub fn find_pane_id_by_uuid(&self, target_uuid: &str) -> Option<u64> {
        match self {
            SplitNode::Leaf { uuid, pane_id, .. } | SplitNode::Preview { uuid, pane_id, .. } => {
                if uuid.to_string() == target_uuid {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => start
                .find_pane_id_by_uuid(target_uuid)
                .or_else(|| end.find_pane_id_by_uuid(target_uuid)),
        }
    }

    /// Set has_attention on the leaf matching pane_id. Returns true if found.
    pub fn set_attention(&mut self, target_pane_id: u64, value: bool) -> bool {
        match self {
            SplitNode::Leaf {
                pane_id,
                has_attention,
                ..
            } => {
                if *pane_id == target_pane_id {
                    *has_attention = value;
                    true
                } else {
                    false
                }
            }
            SplitNode::Preview { .. } => false, // No attention state
            SplitNode::Split { start, end, .. } => {
                start.set_attention(target_pane_id, value)
                    || end.set_attention(target_pane_id, value)
            }
        }
    }

    /// Returns true if any leaf in this subtree has attention.
    pub fn any_attention(&self) -> bool {
        match self {
            SplitNode::Leaf { has_attention, .. } => *has_attention,
            SplitNode::Preview { .. } => false,
            SplitNode::Split { start, end, .. } => start.any_attention() || end.any_attention(),
        }
    }

    /// Check if a specific pane has attention.
    pub fn pane_has_attention(&self, target_pane_id: u64) -> bool {
        match self {
            SplitNode::Leaf {
                pane_id,
                has_attention,
                ..
            } => *pane_id == target_pane_id && *has_attention,
            SplitNode::Preview { .. } => false,
            SplitNode::Split { start, end, .. } => {
                start.pane_has_attention(target_pane_id) || end.pane_has_attention(target_pane_id)
            }
        }
    }

    /// Clear attention on all leaves in this subtree.
    pub fn clear_all_attention(&mut self) {
        match self {
            SplitNode::Leaf { has_attention, .. } => *has_attention = false,
            SplitNode::Preview { .. } => {} // No attention state
            SplitNode::Split { start, end, .. } => {
                start.clear_all_attention();
                end.clear_all_attention();
            }
        }
    }
}

/// Attach right-click context menu to a terminal GLArea (D-08).
/// Uses button 3 (right-click only) to avoid interfering with Ghostty's mouse handling.
fn attach_terminal_context_menu(gl_area: &gtk4::GLArea) {
    let menu_model = crate::menus::build_terminal_context_menu();
    let popover = gtk4::PopoverMenu::from_model(Some(&menu_model));
    popover.set_parent(gl_area);
    popover.set_has_arrow(false);

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3); // Right-click only
    gesture.connect_released({
        let popover = popover.clone();
        move |_, _, x, y| {
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        }
    });
    gl_area.add_controller(gesture);
}

/// SplitEngine manages one workspace's pane layout tree.
pub struct SplitEngine {
    pub root: SplitNode,
    pub active_pane_id: u64,
    /// Monotonically increasing pane ID counter.
    next_pane_id: u64,
    /// GTK Application handle needed to create new GLAreas.
    app: gtk4::Application,
    /// Ghostty app handle needed to create new surfaces.
    ghostty_app: ffi::ghostty_app_t,
}

impl SplitEngine {
    /// Create a new SplitEngine with a single leaf pane.
    /// The initial GLArea and surface are created by the caller (Plan 04) and passed in.
    pub fn new(
        app: gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        initial_gl_area: gtk4::GLArea,
        _initial_surface_cell: std::rc::Rc<std::cell::RefCell<Option<ffi::ghostty_surface_t>>>,
        pane_id: u64,
    ) -> Self {
        // The initial surface may not be realized yet. SplitEngine stores the cell
        // so it can read the surface pointer after realize. For focus/split operations
        // that run after realize, we read from the cell.
        // For the tree structure, we store null initially and update after realize.
        let surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&initial_gl_area);
        let root = SplitNode::Leaf {
            pane_id,
            gl_area: initial_gl_area,
            surface: surface_placeholder,
            uuid: Uuid::new_v4(),
            has_attention: false,
        };
        SplitEngine {
            root,
            active_pane_id: pane_id,
            next_pane_id: pane_id + 1,
            app,
            ghostty_app,
        }
    }

    /// Update the surface pointer for a leaf after realize.
    /// Recursively searches the tree (D-10: works for ALL leaves, not just root).
    #[allow(dead_code)] // kept: session-restore/debug API surface
    pub fn set_initial_surface(&mut self, pane_id: u64, surface: ffi::ghostty_surface_t) {
        Self::set_surface_recursive(&mut self.root, pane_id, surface);
    }

    #[allow(dead_code)]
    fn set_surface_recursive(
        node: &mut SplitNode,
        target_pane_id: u64,
        surface: ffi::ghostty_surface_t,
    ) {
        match node {
            SplitNode::Leaf {
                pane_id,
                surface: s,
                ..
            } => {
                if *pane_id == target_pane_id {
                    *s = surface;
                }
            }
            SplitNode::Preview { .. } => {} // No surface to set
            SplitNode::Split { start, end, .. } => {
                Self::set_surface_recursive(start, target_pane_id, surface);
                Self::set_surface_recursive(end, target_pane_id, surface);
            }
        }
    }

    /// Reconstruct a SplitEngine from serialized SplitNodeData (D-05).
    /// Creates GTK widgets (GLArea for leaves, Paned for splits) and Ghostty surfaces.
    /// Fresh pane_ids are generated (D-06), but UUIDs are preserved from session.
    /// Returns None if tree_depth > 16 (D-14).
    pub fn from_data(
        app: gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        data: &SplitNodeData,
        active_pane_uuid: Option<&str>,
    ) -> Option<Self> {
        let mut next_pane_id: u64 = 1;
        let root = Self::node_from_data(&app, ghostty_app, data, &mut next_pane_id, 0)?;
        // Find active pane by saved UUID, or fall back to first leaf
        let active_id = active_pane_uuid
            .and_then(|uuid_str| root.find_pane_id_by_uuid(uuid_str))
            .unwrap_or_else(|| {
                let mut leaves = Vec::new();
                collect_leaves_in_order(&root, &mut leaves);
                leaves.first().copied().unwrap_or(1)
            });
        Some(SplitEngine {
            root,
            active_pane_id: active_id,
            next_pane_id,
            app,
            ghostty_app,
        })
    }

    fn node_from_data(
        app: &gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        data: &SplitNodeData,
        next_pane_id: &mut u64,
        depth: u32,
    ) -> Option<SplitNode> {
        if depth > 16 {
            tracing::debug!("cmux: session restore tree depth > 16, falling back (D-14)");
            return None;
        }
        match data {
            SplitNodeData::Leaf {
                surface_uuid,
                agent_provider,
                agent_session_id,
                cwd,
                ..
            } => {
                let pane_id = *next_pane_id;
                *next_pane_id += 1;
                // Create surface in the leaf's saved directory (session
                // restore, declarative layouts, and agent surfaces all flow
                // through here). Shell starts there, so splits off it inherit
                // the right cwd and agents find their per-project session.
                let leaf_cwd = if cwd.is_empty() {
                    None
                } else {
                    Some(cwd.clone())
                };
                let (gl_area, _surface_cell) = crate::ghostty::surface::create_surface(
                    app,
                    ghostty_app,
                    None,
                    pane_id,
                    crate::ghostty::surface::SurfaceIoMode::Exec,
                    leaf_cwd,
                );
                // Phase 9: Attach right-click context menu (D-08)
                attach_terminal_context_menu(&gl_area);
                // D-06: preserve UUID from session
                let uuid = *surface_uuid;
                // Re-register agent surfaces so resume + re-save survive restart.
                if let Some(p) = agent_provider
                    .as_deref()
                    .and_then(crate::agent::Provider::from_str)
                {
                    let resume_cwd = if cwd.is_empty() {
                        None
                    } else {
                        Some(cwd.clone())
                    };
                    crate::agent::register(
                        &uuid.to_string(),
                        p,
                        agent_session_id.clone(),
                        resume_cwd,
                    );
                }
                let surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
                Some(SplitNode::Leaf {
                    pane_id,
                    gl_area,
                    surface: surface_placeholder,
                    uuid,
                    has_attention: false,
                })
            }
            SplitNodeData::Split {
                orientation,
                ratio,
                start,
                end,
            } => {
                let start_node =
                    Self::node_from_data(app, ghostty_app, start, next_pane_id, depth + 1)?;
                let end_node =
                    Self::node_from_data(app, ghostty_app, end, next_pane_id, depth + 1)?;
                let gtk_orientation = match orientation.as_str() {
                    "vertical" => gtk4::Orientation::Vertical,
                    _ => gtk4::Orientation::Horizontal,
                };
                let paned = gtk4::Paned::new(gtk_orientation);
                paned.set_resize_start_child(true);
                paned.set_resize_end_child(true);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);
                paned.set_wide_handle(true);
                paned.set_start_child(Some(&start_node.widget()));
                paned.set_end_child(Some(&end_node.widget()));
                // D-03: restore ratio after layout pass
                let saved_ratio = *ratio;
                let paned_ref = paned.clone();
                let orient = gtk_orientation;
                gtk4::glib::idle_add_local_once(move || {
                    let size = if orient == gtk4::Orientation::Horizontal {
                        paned_ref.width()
                    } else {
                        paned_ref.height()
                    };
                    if size > 0 {
                        paned_ref.set_position((size as f64 * saved_ratio) as i32);
                    }
                });
                Some(SplitNode::Split {
                    orientation: gtk_orientation,
                    paned,
                    start: Box::new(start_node),
                    end: Box::new(end_node),
                })
            }
        }
    }

    /// Sync null surface pointers in the tree from GL_TO_SURFACE registry.
    /// Called after restore to wire surfaces that were created during GLArea realize.
    pub fn sync_surfaces_from_registry(&mut self) {
        Self::sync_surfaces_recursive(&mut self.root);
    }

    fn sync_surfaces_recursive(node: &mut SplitNode) {
        match node {
            SplitNode::Leaf {
                gl_area, surface, ..
            } => {
                if surface.is_null() {
                    if let Ok(gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
                        if let Some(&s) = gl_to_surface.get(&(gl_area.as_ptr() as usize)) {
                            *surface = s as ffi::ghostty_surface_t;
                        }
                    }
                }
            }
            SplitNode::Preview { .. } => {} // No surface to sync
            SplitNode::Split { start, end, .. } => {
                Self::sync_surfaces_recursive(start);
                Self::sync_surfaces_recursive(end);
            }
        }
    }

    /// Returns the root widget of this workspace's split tree.
    pub fn root_widget(&self) -> gtk4::Widget {
        self.root.widget()
    }

    /// Grab GTK keyboard focus for the active pane's GLArea.
    /// Called after workspace switch so key events route to Ghostty, not the sidebar.
    pub fn grab_active_focus(&self) {
        if let Some(gl_area) = self.find_gl_area(self.active_pane_id) {
            gl_area.grab_focus();
        }
    }

    /// Returns the UUID of the currently active pane, if found.
    pub fn active_pane_uuid(&self) -> Option<String> {
        self.root.find_uuid_for_pane(self.active_pane_id)
    }

    /// Grab GTK keyboard focus AND notify Ghostty of focus for the active pane.
    /// Use this after any operation that may have moved focus away from the terminal
    /// (sidebar toggle, workspace switch, etc.). grab_active_focus() only handles the
    /// GTK side; this method ensures Ghostty's internal focused state is also updated.
    pub fn focus_active_surface(&self) {
        if let Some(gl_area) = self.find_gl_area(self.active_pane_id) {
            gl_area.grab_focus();
        }
        // Call ghostty_surface_set_focus(true) on the active surface via registry lookup.
        if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
            if let Ok(gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
                for area_ptr in areas.iter() {
                    let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                        unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                    if area.has_css_class("active-pane") {
                        if let Some(&surface_ptr) = gl_to_surface.get(&(area_ptr.0 as usize)) {
                            unsafe {
                                crate::ghostty::ffi::ghostty_surface_set_focus(
                                    surface_ptr as ffi::ghostty_surface_t,
                                    true,
                                );
                            }
                        }
                        break;
                    }
                }
            }
        }
        // Kick the render loop to repaint after focus restore.
        if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
            for area_ptr in areas.iter() {
                let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                if area.is_realized() {
                    area.queue_render();
                }
            }
        }
    }

    /// Split the active pane to the right (Ctrl+D per D-10).
    /// Replaces the active Leaf with a Split(Horizontal) containing the old leaf + new leaf.
    /// Per D-08: new surface inherits CWD via ghostty_surface_inherited_config.
    /// Per D-09: initial split ratio is 50/50 (set in paned.connect_realize).
    /// Per SPLIT-07: new pane receives focus immediately.
    pub fn split_right(&mut self) -> Option<u64> {
        self.split_active(gtk4::Orientation::Horizontal)
    }

    /// Split the active pane downward (Ctrl+Shift+D per D-10).
    pub fn split_down(&mut self) -> Option<u64> {
        self.split_active(gtk4::Orientation::Vertical)
    }

    /// Fibonacci/spiral auto-split: no orientation argument. Orientation is
    /// decided from the *target pane's own* on-screen aspect ratio — split
    /// along its longer axis (wide pane -> vertical divider producing
    /// left/right halves; tall pane -> horizontal divider producing
    /// top/bottom halves). This is what makes repeated spawning look like a
    /// spiral: each new pane is narrower/shorter than its parent, so it
    /// naturally alternates axes as the tree grows.
    ///
    /// Deliberately *not* a global per-workspace counter: an earlier version
    /// alternated orientation by a shared call count, which meant refocusing
    /// an already-split pane and spawning again picked up wherever the
    /// counter had drifted to from splits made elsewhere — inconsistent from
    /// the caller's point of view. Deciding from the pane's own geometry
    /// means splitting the same pane always behaves the same way, regardless
    /// of what else happened in the workspace.
    ///
    /// Used by agentic orchestration: an orchestrator/lead/worker fan-out can
    /// call this repeatedly with zero layout bookkeeping of its own.
    pub fn spiral_orientation_for(&self, pane_id: u64) -> gtk4::Orientation {
        match self.pane_size(pane_id) {
            // Wider than tall (or square) -> vertical divider (left/right).
            // Taller than wide -> horizontal divider (top/bottom).
            Some((w, h)) if h > w => gtk4::Orientation::Vertical,
            _ => gtk4::Orientation::Horizontal,
        }
    }

    pub fn spiral_split(&mut self) -> Option<u64> {
        let orientation = self.spiral_orientation_for(self.active_pane_id);
        self.split_active(orientation)
    }

    pub fn split_active(&mut self, orientation: gtk4::Orientation) -> Option<u64> {
        let active_id = self.active_pane_id;
        let new_pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        // When the root is a Leaf (first split), the GLArea is a direct child of the GtkStack
        // page. The replacer will remove it from the Stack (via remove_widget_from_parent) and
        // place it inside the new Paned. We then need to add the Paned to the Stack page.
        // Only capture this for Leaf roots — for nested splits the outer Paned stays in the Stack.
        let old_root_widget = self.root.widget();
        let stack_slot: Option<(gtk4::Stack, String)> = if matches!(
            self.root,
            SplitNode::Leaf { .. } | SplitNode::Preview { .. }
        ) {
            old_root_widget
                .parent()
                .and_then(|p| p.downcast::<gtk4::Stack>().ok())
                .and_then(|stack| {
                    let name = stack.page(&old_root_widget).name()?.to_string();
                    Some((stack, name))
                })
        } else {
            None
        };

        // Find the active leaf's surface for inherited config.
        let inherited_surface = self.find_surface(active_id)?;

        // Unfocus the old surface before the split — Ghostty routes input by focus state,
        // so without this the old pane continues receiving keystrokes after the new pane
        // is created (SPLIT-07).
        unsafe {
            ffi::ghostty_surface_set_focus(inherited_surface, false);
        }

        // Get inherited config from the active surface (for CWD inheritance per D-08).
        // Pass by value (ghostty_surface_config_s is Copy) — avoids dangling pointer
        // in the GLArea realize callback, which fires asynchronously after this returns.
        let inherited_config = unsafe {
            ffi::ghostty_surface_inherited_config(
                inherited_surface,
                ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_SPLIT,
            )
        };

        // Create new GLArea + surface for the new pane.
        tracing::debug!(
            "cmux: split_active calling create_surface for new_pane_id={}",
            new_pane_id
        );
        let (new_gl_area, _surface_cell) = crate::ghostty::surface::create_surface(
            &self.app,
            self.ghostty_app,
            Some(inherited_config),
            new_pane_id,
            crate::ghostty::surface::SurfaceIoMode::Exec,
            // cwd inherited via inherited_config (parent starts in the right
            // directory now, so ghostty_surface_inherited_config carries it).
            None,
        );
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&new_gl_area);

        // Replace the active leaf in the tree with a Split node.
        let new_surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
        let new_leaf = SplitNode::Leaf {
            pane_id: new_pane_id,
            gl_area: new_gl_area.clone(),
            surface: new_surface_placeholder, // updated after realize via SURFACE_REGISTRY
            uuid: Uuid::new_v4(),
            has_attention: false,
        };

        self.replace_leaf_with_split(active_id, new_leaf, orientation)?;

        // If the root was a Leaf, it's now a Split whose Paned has no parent.
        // Re-parent the new Paned root into the GtkStack page we saved above.
        if let Some((stack, name)) = stack_slot {
            let new_root = self.root.widget();
            stack.add_named(&new_root, Some(&name));
            stack.set_visible_child_name(&name);
        }

        // After realize, update active focus to the new pane.
        self.active_pane_id = new_pane_id;
        self.root.update_focus_css(new_pane_id);

        // Focus the new GLArea widget so it receives keyboard events.
        new_gl_area.grab_focus();

        Some(new_pane_id)
    }

    /// Allocate the next available pane ID (used by browser preview pane creation).
    #[allow(dead_code)]
    pub fn allocate_pane_id(&mut self) -> u64 {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        id
    }

    /// Split the active pane vertically and insert a Preview node on the right.
    /// Returns (new_pane_id, picture_widget, url_entry) so the caller can wire streaming and URL navigation.
    /// The active terminal pane stays on the left and retains focus.
    pub fn split_active_with_preview(&mut self) -> Option<crate::browser::PreviewPaneWidgets> {
        let active_id = self.active_pane_id;
        let new_pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        // Same stack re-parenting guard as split_active:
        // When root is a Leaf or Preview, capture the GtkStack parent so we can
        // re-parent the new Paned into the Stack after the split.
        let old_root_widget = self.root.widget();
        let stack_slot: Option<(gtk4::Stack, String)> = if matches!(
            self.root,
            SplitNode::Leaf { .. } | SplitNode::Preview { .. }
        ) {
            old_root_widget
                .parent()
                .and_then(|p| p.downcast::<gtk4::Stack>().ok())
                .and_then(|stack| {
                    let name = stack.page(&old_root_widget).name()?.to_string();
                    Some((stack, name))
                })
        } else {
            None
        };

        // Create preview pane widgets
        let widgets = crate::browser::create_preview_pane(new_pane_id);

        // Phase 9: Attach right-click context menu to browser preview (D-09)
        {
            let menu_model = crate::menus::build_browser_context_menu();
            let popover = gtk4::PopoverMenu::from_model(Some(&menu_model));
            popover.set_parent(&widgets.container);
            popover.set_has_arrow(false);

            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // Right-click only
            gesture.connect_released({
                let popover = popover.clone();
                move |_, _, x, y| {
                    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                        x as i32, y as i32, 1, 1,
                    )));
                    popover.popup();
                }
            });
            widgets.container.add_controller(gesture);
        }

        let preview_node = SplitNode::Preview {
            pane_id: new_pane_id,
            container: widgets.container.clone(),
            picture: widgets.picture.clone(),
            url_entry: widgets.url_entry.clone(),
            uuid: widgets.uuid,
        };

        // Replace active leaf with Split(active_leaf, preview_node) -- vertical, preview on right
        self.replace_leaf_with_split(
            active_id,
            preview_node,
            gtk4::Orientation::Horizontal, // Horizontal paned = side-by-side (left terminal, right preview)
        )?;

        // Re-parent new root Paned into GtkStack if root was a single Leaf
        if let Some((stack, name)) = stack_slot {
            let new_root = self.root.widget();
            stack.add_named(&new_root, Some(&name));
            stack.set_visible_child_name(&name);
        }

        // Keep focus on the original terminal pane (do NOT change active_pane_id)
        // The terminal keeps keyboard input, preview is passive display only.
        self.root.update_focus_css(active_id);

        Some(widgets)
    }

    /// Replace the leaf with `target_pane_id` with a Split(orientation) node.
    /// Returns Some(()) on success, None if the leaf was not found.
    fn replace_leaf_with_split(
        &mut self,
        target_pane_id: u64,
        new_leaf: SplitNode,
        orientation: gtk4::Orientation,
    ) -> Option<()> {
        let orientation_cap = orientation;
        let mut replacer = Some(|old_leaf: SplitNode| {
            let old_widget = old_leaf.widget();
            let new_widget = new_leaf.widget();

            // GTK4 requires a widget to have no parent before set_start/end_child.
            // old_widget may be parented to the Stack (first split) or an outer Paned (nested).
            remove_widget_from_parent(&old_widget);

            let paned = gtk4::Paned::new(orientation_cap);
            // Both children must be allowed to resize — GTK4 default for resize_end_child
            // is TRUE but be explicit to ensure drag works in both directions.
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            // Prevent children from collapsing to 0px when dragging to an extreme.
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            // Wide handle makes the divider grabable (default is ~5px, hard to click).
            paned.set_wide_handle(true);

            paned.set_start_child(Some(&old_widget));
            paned.set_end_child(Some(&new_widget));

            // Set 50/50 position after the first layout pass (per D-09 and RESEARCH Pitfall 2).
            // connect_realize fires before GTK allocates sizes, so p.width() is 0 there.
            // idle_add_local_once defers to the next main-loop idle, after layout completes.
            {
                let paned_ref = paned.clone();
                gtk4::glib::idle_add_local_once(move || {
                    let size = if orientation_cap == gtk4::Orientation::Horizontal {
                        paned_ref.width()
                    } else {
                        paned_ref.height()
                    };
                    if size > 0 {
                        paned_ref.set_position(size / 2);
                    }
                });
            }

            // Restore GTK focus AND Ghostty focus after GtkPaned drag ends (Gap 1A).
            //
            // The divider drag temporarily moves GTK focus to the separator handle,
            // which causes Ghostty's cursor blink and keyboard input to stop.
            //
            // IMPORTANT: We must NOT restore focus on every `notify::position` change
            // during the drag — that fires on every pixel of movement and causes:
            //   1. grab_focus() thrashing that fights the active gesture
            //   2. ghostty_surface_set_focus(true) message storms to the render thread
            //   3. GL_AREA_REGISTRY lock contention with the resize idle handler
            // Instead, we detect the drag lifecycle via the paned's internal GestureDrag
            // controller and restore focus only once when the drag ends.
            {
                // Find the GtkPaned's internal GestureDrag on its separator handle.
                // GTK4's Paned uses a GestureDrag controller internally — we observe
                // the controller list and connect to its drag-end signal.
                //
                // The gesture lives on the separator handle widget, not the Paned
                // itself. Walk the Paned's children to find the separator, then
                // inspect its controllers.
                let mut found_gesture = false;

                // First try: controllers on the Paned itself
                let controllers = paned.observe_controllers();
                let n = controllers.n_items();
                tracing::debug!("cmux: Paned has {} controllers", n);
                for i in 0..n {
                    if let Some(obj) = controllers.item(i) {
                        tracing::debug!("cmux:   controller[{}]: {}", i, obj.type_().name());
                        if let Ok(gesture) = obj.downcast::<gtk4::GestureDrag>() {
                            tracing::debug!(
                                "cmux:   -> found GestureDrag on Paned, connecting drag-end"
                            );
                            gesture.connect_drag_end(|_gesture, _offset_x, _offset_y| {
                                tracing::debug!("cmux: GestureDrag drag-end fired on Paned — deferring focus restore to idle");
                                // Defer to idle so GTK has time to fully release the gesture
                                // and clean up the event sequence before we move focus.
                                gtk4::glib::idle_add_local_once(|| {
                                    tracing::debug!("cmux: drag-end idle: restoring focus now");
                                    restore_active_pane_focus();
                                });
                            });
                            found_gesture = true;
                            break;
                        }
                    }
                }

                // Second try: walk children to find the separator handle widget
                if !found_gesture {
                    let mut child = paned.first_child();
                    while let Some(ref widget) = child {
                        let type_name = widget.type_().name();
                        tracing::debug!("cmux: Paned child: {}", type_name);
                        let ctrl_list = widget.observe_controllers();
                        let cn = ctrl_list.n_items();
                        for i in 0..cn {
                            if let Some(obj) = ctrl_list.item(i) {
                                tracing::debug!(
                                    "cmux:   child controller[{}]: {}",
                                    i,
                                    obj.type_().name()
                                );
                                if let Ok(gesture) = obj.downcast::<gtk4::GestureDrag>() {
                                    tracing::debug!(
                                        "cmux:   -> found GestureDrag on {}, connecting drag-end",
                                        type_name
                                    );
                                    gesture.connect_drag_end(|_gesture, _offset_x, _offset_y| {
                                        tracing::debug!("cmux: GestureDrag drag-end fired on separator — deferring focus restore to idle");
                                        gtk4::glib::idle_add_local_once(|| {
                                            tracing::debug!("cmux: separator drag-end idle: restoring focus now");
                                            restore_active_pane_focus();
                                        });
                                    });
                                    found_gesture = true;
                                    break;
                                }
                            }
                        }
                        if found_gesture {
                            break;
                        }
                        child = widget.next_sibling();
                    }
                }

                if !found_gesture {
                    tracing::debug!("cmux: WARNING — no GestureDrag found on Paned or its children, falling back to notify::position");
                    // Fallback: use notify::position but debounced via idle.
                    // connect_notify requires Send+Sync, so use AtomicBool instead of Rc<Cell>.
                    let restore_pending =
                        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    paned.connect_notify(Some("position"), move |_paned, _pspec| {
                        if restore_pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            return;
                        }
                        let pending = restore_pending.clone();
                        gtk4::glib::idle_add_once(move || {
                            pending.store(false, std::sync::atomic::Ordering::SeqCst);
                            restore_active_pane_focus();
                        });
                    });
                }
            }

            SplitNode::Split {
                orientation: orientation_cap,
                paned: paned.clone(),
                start: Box::new(old_leaf),
                end: Box::new(new_leaf),
            }
        });
        replace_in_tree(&mut self.root, target_pane_id, &mut replacer)
    }

    /// Close the active pane (Ctrl+Shift+X per UI-SPEC).
    /// Removes the active leaf, replaces its parent Split with the surviving sibling.
    /// Returns the new active pane_id, or None if this was the last pane.
    pub fn close_active(&mut self) -> Option<u64> {
        let active_id = self.active_pane_id;

        // Cannot close the last pane — workspace close is handled at AppState level.
        let is_single_pane = match &self.root {
            SplitNode::Leaf { pane_id, .. } if *pane_id == active_id => true,
            SplitNode::Preview { pane_id, .. } if *pane_id == active_id => true,
            _ => false,
        };
        if is_single_pane {
            return None; // Signal to AppState: close the workspace instead
        }

        // Don't close the last terminal pane if only a Preview would survive.
        // A Preview-only workspace has no terminal and the Ghostty surface free
        // can crash when no terminal remains to receive focus.
        let terminal_count = count_terminals(&self.root);
        let active_is_terminal =
            matches!(self.root.find_node(active_id), Some(SplitNode::Leaf { .. }));
        if active_is_terminal && terminal_count <= 1 {
            return None; // Prevent closing last terminal; close workspace instead
        }

        // Capture the raw GLArea pointer BEFORE the tree removal drops the GObject.
        // GL_AREA_REGISTRY holds raw pointers; once GTK finalizes the GObject the
        // pointer becomes dangling. Remove it here while the GLArea is still alive.
        let raw_gl_area: Option<*mut gtk4::ffi::GtkGLArea> =
            self.find_gl_area(active_id).map(|a| a.as_ptr());

        // Remove the leaf from the tree and get the surviving sibling's pane_id.
        let surviving_id = remove_leaf_from_tree(&mut self.root, active_id)?;

        // Remove the now-dropped GLArea from GL_AREA_REGISTRY before any further
        // callbacks can dereference the dangling pointer (Gap 2 fix).
        if let Some(raw_ptr) = raw_gl_area {
            if let Ok(mut areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                areas.retain(|p| p.0 != raw_ptr);
            }
            // Also remove from GL_TO_SURFACE mapping.
            if let Ok(mut gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
                gl_to_surface.remove(&(raw_ptr as usize));
            }
        }

        // The ghostty surface is NOT freed here: the GLArea unrealize handler
        // owns it (realize creates, unrealize frees + deregisters), and the
        // tree removal above unparented the GLArea, which unrealizes it.
        // Freeing here too double-frees whenever the tree pointer is live
        // (e.g. after sync_surfaces_from_registry on session restore).

        // Update focus to the surviving pane.
        self.active_pane_id = surviving_id;
        self.root.update_focus_css(surviving_id);

        // Call ghostty_surface_set_focus on the surviving surface (SPLIT-07).
        if let Some(surface) = self.find_surface(surviving_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(surface, true);
            }
        }

        // Grab GTK focus on the surviving pane's widget (GLArea or URL entry).
        if let Some(gl_area) = self.find_gl_area(surviving_id) {
            gl_area.grab_focus();
        } else if let Some(entry) = find_url_entry_in_tree(&self.root, surviving_id) {
            entry.grab_focus();
        }

        Some(surviving_id)
    }

    /// Navigate focus to the pane adjacent in `direction` (Ctrl+Alt+arrows per D-10).
    pub fn focus_next_in_direction(&mut self, direction: FocusDirection) -> bool {
        let active_id = self.active_pane_id;
        if let Some(new_id) = find_adjacent(&self.root, active_id, direction) {
            // Unfocus old surface.
            if let Some(old_surface) = self.find_surface(active_id) {
                unsafe {
                    ffi::ghostty_surface_set_focus(old_surface, false);
                }
            }
            self.active_pane_id = new_id;
            self.root.update_focus_css(new_id);
            // Focus new surface or Preview URL entry.
            if let Some(new_surface) = self.find_surface(new_id) {
                unsafe {
                    ffi::ghostty_surface_set_focus(new_surface, true);
                }
            }
            if let Some(gl_area) = self.find_gl_area(new_id) {
                gl_area.grab_focus();
            } else if let Some(entry) = find_url_entry_in_tree(&self.root, new_id) {
                entry.grab_focus();
            }
            true
        } else {
            false
        }
    }

    /// Update the surface pointer for a pane after its GLArea realize callback fires.
    /// Called by Plan 04 wiring after SURFACE_REGISTRY is populated.
    #[allow(dead_code)]
    pub fn update_surface(&mut self, pane_id: u64, surface: ffi::ghostty_surface_t) {
        update_surface_in_tree(&mut self.root, pane_id, surface);
    }

    fn find_surface(&self, pane_id: u64) -> Option<ffi::ghostty_surface_t> {
        find_surface_in_tree(&self.root, pane_id).or_else(|| {
            // Fallback: look up in global SURFACE_REGISTRY by scanning for pane_id.
            // SURFACE_REGISTRY maps surface_ptr (usize) → pane_id; need reverse lookup.
            if let Ok(reg) = crate::ghostty::callbacks::SURFACE_REGISTRY.lock() {
                reg.iter()
                    .find(|(_, &pid)| pid == pane_id)
                    .map(|(&ptr, _)| ptr as ffi::ghostty_surface_t)
            } else {
                None
            }
        })
    }

    fn find_gl_area(&self, pane_id: u64) -> Option<gtk4::GLArea> {
        find_gl_area_in_tree(&self.root, pane_id)
    }

    /// Current on-screen size (logical px) of a pane's terminal widget, if it
    /// is realized and allocated. Used to reject splits that would produce a
    /// pane too small for its shell/TUI to render.
    pub fn pane_size(&self, pane_id: u64) -> Option<(i32, i32)> {
        use gtk4::prelude::*;
        let area = self.find_gl_area(pane_id)?;
        let (w, h) = (area.width(), area.height());
        if w > 0 && h > 0 {
            Some((w, h))
        } else {
            None
        }
    }

    /// Returns all leaf panes in this engine as (uuid, pane_id, active) tuples.
    pub fn all_panes(&self) -> Vec<(Uuid, u64, bool)> {
        let mut panes = Vec::new();
        self.root.collect_pane_info(&mut panes, self.active_pane_id);
        panes
    }

    /// Look up a surface by its UUID string. Returns the ghostty surface handle if found.
    #[allow(dead_code)]
    pub fn find_surface_by_uuid(&self, target_uuid: &str) -> Option<ffi::ghostty_surface_t> {
        self.root.find_by_uuid(target_uuid)
    }

    /// Look up a pane_id by its UUID string.
    pub fn find_pane_id_by_uuid(&self, target_uuid: &str) -> Option<u64> {
        self.root.find_pane_id_by_uuid(target_uuid)
    }

    /// Look up a GLArea by pane_id (public wrapper for socket handlers).
    pub fn gl_area_for_pane(&self, pane_id: u64) -> Option<gtk4::GLArea> {
        find_gl_area_in_tree(&self.root, pane_id)
    }
}

// ── Tree traversal helpers ───────────────────────────────────────────────────

/// Replace the leaf with `target_id` using `replacer` function. Returns Some(()) if found.
fn replace_in_tree<F>(node: &mut SplitNode, target_id: u64, replacer: &mut Option<F>) -> Option<()>
where
    F: FnOnce(SplitNode) -> SplitNode,
{
    match node {
        SplitNode::Leaf { pane_id, .. } if *pane_id == target_id => {
            if let Some(r) = replacer.take() {
                // Take ownership of the old node to pass to replacer.
                let old = std::mem::replace(
                    node,
                    SplitNode::Leaf {
                        pane_id: 0,
                        gl_area: gtk4::GLArea::new(),
                        surface: std::ptr::null_mut(),
                        uuid: Uuid::new_v4(),
                        has_attention: false,
                    },
                );
                *node = r(old);
                Some(())
            } else {
                None
            }
        }
        SplitNode::Leaf { .. } => None,
        SplitNode::Preview { .. } => None, // Cannot split a preview pane
        SplitNode::Split {
            start, end, paned, ..
        } => {
            if let Some(()) = replace_in_tree(start, target_id, replacer) {
                // Update paned start child to new widget.
                paned.set_start_child(Some(&start.widget()));
                Some(())
            } else if let Some(()) = replace_in_tree(end, target_id, replacer) {
                paned.set_end_child(Some(&end.widget()));
                Some(())
            } else {
                None
            }
        }
    }
}

/// Remove leaf `target_id` from the tree. Returns the surviving sibling's pane_id.
/// Replaces the parent Split with the surviving sibling in the GTK widget tree.
fn remove_leaf_from_tree(node: &mut SplitNode, target_id: u64) -> Option<u64> {
    match node {
        SplitNode::Leaf { .. } | SplitNode::Preview { .. } => None, // Caller ensures we never remove the root leaf
        SplitNode::Split {
            start, end, paned, ..
        } => {
            // Check if start is the target (Leaf or Preview).
            let start_is_target = match start.as_ref() {
                SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => {
                    *pane_id == target_id
                }
                _ => false,
            };
            if start_is_target {
                // Surviving sibling is end. Replace this Split with end in the GTK tree.
                let surviving = *end.clone();
                let surviving_widget = surviving.widget();
                // Find the paned's parent and replace it with the surviving widget.
                if let Some(parent) = paned.parent() {
                    replace_child_in_parent(&parent, &paned.clone().upcast(), &surviving_widget);
                }
                let surviving_id = first_pane_id(&surviving);
                *node = surviving;
                return Some(surviving_id);
            }
            // Check if end is the target (Leaf or Preview).
            let end_is_target = match end.as_ref() {
                SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => {
                    *pane_id == target_id
                }
                _ => false,
            };
            if end_is_target {
                let surviving = *start.clone();
                let surviving_widget = surviving.widget();
                if let Some(parent) = paned.parent() {
                    replace_child_in_parent(&parent, &paned.clone().upcast(), &surviving_widget);
                }
                let surviving_id = first_pane_id(&surviving);
                *node = surviving;
                return Some(surviving_id);
            }
            // Recurse into start subtree.
            if let Some(id) = remove_leaf_from_tree(start, target_id) {
                paned.set_start_child(Some(&start.widget()));
                return Some(id);
            }
            // Recurse into end subtree.
            if let Some(id) = remove_leaf_from_tree(end, target_id) {
                paned.set_end_child(Some(&end.widget()));
                return Some(id);
            }
            None
        }
    }
}

/// Replace `old_widget` with `new_widget` in `parent`. Handles GtkPaned children and GtkStack pages.
fn replace_child_in_parent(
    parent: &gtk4::Widget,
    old_widget: &gtk4::Widget,
    new_widget: &gtk4::Widget,
) {
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        if paned
            .start_child()
            .as_ref()
            .map(|w| w == old_widget)
            .unwrap_or(false)
        {
            paned.set_start_child(Some(new_widget));
        } else {
            paned.set_end_child(Some(new_widget));
        }
    } else if let Some(stack) = parent.downcast_ref::<gtk4::Stack>() {
        let page = stack.page(old_widget);
        if let Some(name) = page.name() {
            let name_str = name.to_string();
            stack.remove(old_widget);
            // new_widget may still be parented to the Paned we're replacing; unparent first.
            remove_widget_from_parent(new_widget);
            stack.add_named(new_widget, Some(&name_str));
            stack.set_visible_child_name(&name_str);
        } else {
            stack.remove(old_widget);
        }
    }
    // If parent is something else, the widget swap is a no-op (should not happen in Phase 2).
}

/// Return the first (leftmost/topmost) pane_id in a subtree.
fn first_pane_id(node: &SplitNode) -> u64 {
    match node {
        SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => *pane_id,
        SplitNode::Split { start, .. } => first_pane_id(start),
    }
}

fn find_surface_in_tree(node: &SplitNode, pane_id: u64) -> Option<ffi::ghostty_surface_t> {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            surface,
            ..
        } if *id == pane_id => {
            if surface.is_null() {
                None
            } else {
                Some(*surface)
            }
        }
        SplitNode::Leaf { .. } => None,
        SplitNode::Preview { .. } => None, // No Ghostty surface
        SplitNode::Split { start, end, .. } => {
            find_surface_in_tree(start, pane_id).or_else(|| find_surface_in_tree(end, pane_id))
        }
    }
}

fn find_gl_area_in_tree(node: &SplitNode, pane_id: u64) -> Option<gtk4::GLArea> {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            gl_area,
            ..
        } if *id == pane_id => Some(gl_area.clone()),
        SplitNode::Leaf { .. } => None,
        SplitNode::Preview { .. } => None, // Preview uses Picture, not GLArea
        SplitNode::Split { start, end, .. } => {
            find_gl_area_in_tree(start, pane_id).or_else(|| find_gl_area_in_tree(end, pane_id))
        }
    }
}

/// Count terminal (Leaf) panes in the tree.
fn count_terminals(node: &SplitNode) -> usize {
    match node {
        SplitNode::Leaf { .. } => 1,
        SplitNode::Preview { .. } => 0,
        SplitNode::Split { start, end, .. } => count_terminals(start) + count_terminals(end),
    }
}

fn find_url_entry_in_tree(node: &SplitNode, pane_id: u64) -> Option<gtk4::Entry> {
    match node {
        SplitNode::Preview {
            pane_id: id,
            url_entry,
            ..
        } if *id == pane_id => Some(url_entry.clone()),
        SplitNode::Preview { .. } | SplitNode::Leaf { .. } => None,
        SplitNode::Split { start, end, .. } => {
            find_url_entry_in_tree(start, pane_id).or_else(|| find_url_entry_in_tree(end, pane_id))
        }
    }
}

#[allow(dead_code)] // kept: session-restore/debug API surface
fn update_surface_in_tree(node: &mut SplitNode, pane_id: u64, surface: ffi::ghostty_surface_t) {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            surface: s,
            ..
        } if *id == pane_id => *s = surface,
        SplitNode::Leaf { .. } => {}
        SplitNode::Preview { .. } => {} // No surface to update
        SplitNode::Split { start, end, .. } => {
            update_surface_in_tree(start, pane_id, surface);
            update_surface_in_tree(end, pane_id, surface);
        }
    }
}

/// Find the pane adjacent to `active_id` in `direction`.
/// Strategy: collect ordered leaf positions and find the neighbor.
/// This is a directional approximation: Left/Up = previous leaf, Right/Down = next leaf.
/// A full spatial algorithm (comparing widget coordinates) can be added in a future phase.
fn find_adjacent(root: &SplitNode, active_id: u64, direction: FocusDirection) -> Option<u64> {
    let mut leaves = Vec::new();
    collect_leaves_in_order(root, &mut leaves);
    let pos = leaves.iter().position(|&id| id == active_id)?;
    match direction {
        FocusDirection::Left | FocusDirection::Up => {
            if pos > 0 {
                Some(leaves[pos - 1])
            } else {
                None
            }
        }
        FocusDirection::Right | FocusDirection::Down => {
            if pos + 1 < leaves.len() {
                Some(leaves[pos + 1])
            } else {
                None
            }
        }
    }
}

/// Remove `widget` from its current GTK parent so it can be reparented.
/// GTK4 requires `gtk_widget_get_parent(child) == NULL` before set_start/end_child.
fn remove_widget_from_parent(widget: &gtk4::Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        if paned
            .start_child()
            .as_ref()
            .map(|w| w == widget)
            .unwrap_or(false)
        {
            paned.set_start_child(None::<&gtk4::Widget>);
        } else {
            paned.set_end_child(None::<&gtk4::Widget>);
        }
    } else if let Some(stack) = parent.downcast_ref::<gtk4::Stack>() {
        stack.remove(widget);
    }
}

/// Restore GTK keyboard focus and Ghostty surface focus to the active pane.
/// Called once when a GtkPaned drag ends — NOT on every pixel of movement.
/// Re-syncs each surface's cached size with the GLArea's current allocation to
/// break any anti-flicker stall in Ghostty's drawFrame() guard, then kicks
/// the render thread with ghostty_surface_refresh + queue_render.
///
/// Does NOT touch focus state. The cursor blink timer runs independently of
/// resize. Calling set_focus(false→true) here kills the timer due to an async
/// cancel race in Ghostty's renderer thread: the false message enqueues a timer
/// cancel, but the true message is processed before the cancel callback fires,
/// so the guard `if cursor_c.state() != .active` sees `.active` and skips the
/// restart. The cancel then completes, leaving the timer permanently dead.
fn restore_active_pane_focus() {
    // Re-set size + refresh ALL surfaces to break the anti-flicker stall.
    if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
        if let Ok(gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
            for area_ptr in areas.iter() {
                let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                if let Some(&surface_ptr) = gl_to_surface.get(&(area_ptr.0 as usize)) {
                    let scale = area.scale_factor();
                    let w = (area.width() * scale) as u32;
                    let h = (area.height() * scale) as u32;
                    if w > 0 && h > 0 {
                        unsafe {
                            let surface = surface_ptr as ffi::ghostty_surface_t;
                            ffi::ghostty_surface_set_size(surface, w, h);
                            ffi::ghostty_surface_refresh(surface);
                        }
                    }
                }
                if area.is_realized() {
                    area.queue_render();
                    area.queue_draw();
                }
            }
        }
    }

    // Drive the app tick to process any pending mailbox messages (redraw_surface, etc.)
    let app_ptr = crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
    if app_ptr != 0 {
        unsafe {
            let app = app_ptr as ffi::ghostty_app_t;
            ffi::ghostty_app_tick(app);
        }
    }

    // Restore GTK keyboard focus to the active pane — but only if focus is
    // not already inside an editable widget (browser URL bar, sidebar rename
    // entry, SSH dialog, …). Stealing focus from those is a UX regression:
    // every divider resize would interrupt typing.
    if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
        // Identify the window the active pane lives in. Picking the first
        // GLArea unconditionally was wrong on multi-window setups — the
        // hash iteration order picked an arbitrary area in some other
        // window, so we queried the wrong window's focused widget.
        let mut active_window: Option<gtk4::Window> = None;
        for area_ptr in areas.iter() {
            let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
            if area.has_css_class("active-pane") {
                if let Some(root) = area.root() {
                    active_window = root.dynamic_cast::<gtk4::Window>().ok();
                    break;
                }
            }
        }

        let mut focus_in_editable = false;
        if let Some(window) = active_window.as_ref() {
            let focused: Option<gtk4::Widget> = gtk4::prelude::GtkWindowExt::focus(window);
            if let Some(focused) = focused {
                if focused.is::<gtk4::Entry>()
                    || focused.is::<gtk4::Text>()
                    || focused.is::<gtk4::SearchEntry>()
                    || focused.is::<gtk4::TextView>()
                {
                    focus_in_editable = true;
                }
            }
        }
        if !focus_in_editable {
            for area_ptr in areas.iter() {
                let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                if area.has_css_class("active-pane") {
                    area.grab_focus();
                    area.queue_render();
                    break;
                }
            }
        }
    }

    // The set_size → IO thread → render thread → updateFrame → cells rebuild pipeline
    // is asynchronous. The immediate queue_render above may still draw stale content
    // because cells haven't been rebuilt yet. Schedule follow-up recovery ticks to
    // give the pipeline time to converge (50ms, 150ms, 300ms).
    for delay_ms in [50u32, 150, 300] {
        gtk4::glib::timeout_add_local_once(
            std::time::Duration::from_millis(delay_ms as u64),
            move || {
                let app_ptr =
                    crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
                if app_ptr != 0 {
                    unsafe {
                        ffi::ghostty_app_tick(app_ptr as ffi::ghostty_app_t);
                    }
                }
                if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                    for area_ptr in areas.iter() {
                        let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                            unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                        if area.is_realized() {
                            area.queue_render();
                        }
                    }
                }
            },
        );
    }
}

fn collect_leaves_in_order(node: &SplitNode, out: &mut Vec<u64>) {
    match node {
        SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => out.push(*pane_id),
        SplitNode::Split { start, end, .. } => {
            collect_leaves_in_order(start, out);
            collect_leaves_in_order(end, out);
        }
    }
}

/// Serde-friendly mirror of SplitNode for session persistence.
/// GTK widget references (GLArea, Paned) cannot be serialized — this parallel type holds
/// only the data needed to reconstruct the tree on restore.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum SplitNodeData {
    Leaf {
        pane_id: u64,
        surface_uuid: Uuid,
        /// Shell executable path, e.g. "/bin/zsh" or "/bin/bash"
        shell: String,
        /// Absolute working directory path (best-effort; may be empty if /proc unavailable)
        cwd: String,
        /// Agent provider running in this pane, if it's a native agent
        /// surface (e.g. "claude"). None for plain shells.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_provider: Option<String>,
        /// Captured native session id for resume, if the agent's hook has
        /// reported one. None until captured.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_session_id: Option<String>,
    },
    Split {
        /// "horizontal" or "vertical"
        orientation: String,
        /// Divider position as fraction 0.0-1.0 relative to parent size (D-03).
        #[serde(default = "default_ratio")]
        ratio: f64,
        start: Box<SplitNodeData>,
        end: Box<SplitNodeData>,
    },
}

fn default_ratio() -> f64 {
    0.5
}

/// Best-effort CWD capture for a Ghostty surface via /proc.
/// Walks /proc looking for child processes of the current process (cmux),
/// then reads /proc/{pid}/cwd for the foreground shell.
/// Never panics — falls back to $HOME or empty string.
fn get_surface_cwd(surface: ffi::ghostty_surface_t) -> String {
    if surface.is_null() {
        return String::new();
    }
    // Try to find child shell processes by scanning /proc for children of our PID.
    // Each Ghostty surface spawns a child shell — we look for processes whose
    // parent is the cmux process and read their CWD.
    let our_pid = std::process::id();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        // Collect candidate child PIDs (children of our process)
        let mut candidates: Vec<u32> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Ok(pid) = name.to_string_lossy().parse::<u32>() {
                // Read /proc/{pid}/stat to check parent PID
                if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                    // Format: pid (comm) state ppid ...
                    // Find the closing paren then parse ppid
                    if let Some(after_comm) = stat.rfind(')') {
                        let fields: Vec<&str> = stat[after_comm + 2..].split_whitespace().collect();
                        if fields.len() >= 2 {
                            if let Ok(ppid) = fields[1].parse::<u32>() {
                                if ppid == our_pid {
                                    candidates.push(pid);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Use the last (most recent) child process CWD as best guess.
        // In practice, each surface has one direct child shell.
        for pid in candidates.iter().rev() {
            if let Ok(cwd) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
                let cwd_str = cwd.to_string_lossy().to_string();
                if !cwd_str.is_empty() {
                    return cwd_str;
                }
            }
        }
    }
    // Fallback to $HOME
    std::env::var("HOME").unwrap_or_default()
}

impl SplitNode {
    /// Produce a serializable snapshot of this node's tree structure.
    /// `shell` and `cwd` are best-effort: Plan 05 fills these via /proc.
    /// Falls back to empty strings if /proc is unavailable or the pid is unknown.
    pub fn to_data(&self) -> SplitNodeData {
        match self {
            SplitNode::Leaf {
                pane_id,
                uuid,
                surface,
                ..
            } => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let agent = crate::agent::get(&uuid.to_string());
                // For agent surfaces, the registry holds the authoritative
                // launch cwd (agents key their session store by project dir,
                // and get_surface_cwd is a best-effort /proc guess that can't
                // reliably map a surface to its own child). Fall back to the
                // /proc guess for plain shells.
                let cwd = agent
                    .as_ref()
                    .and_then(|a| a.cwd.clone())
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| get_surface_cwd(*surface));
                SplitNodeData::Leaf {
                    pane_id: *pane_id,
                    surface_uuid: *uuid,
                    shell,
                    cwd,
                    agent_provider: agent.as_ref().map(|a| a.provider.as_str().to_string()),
                    agent_session_id: agent.and_then(|a| a.session_id),
                }
            }
            SplitNode::Preview { .. } => {
                // Preview panes are ephemeral; skip in session serialization.
                // Return a dummy leaf that will be ignored during restore.
                SplitNodeData::Leaf {
                    pane_id: 0,
                    surface_uuid: Uuid::nil(),
                    shell: String::new(),
                    cwd: String::new(),
                    agent_provider: None,
                    agent_session_id: None,
                }
            }
            SplitNode::Split {
                orientation,
                paned,
                start,
                end,
                ..
            } => {
                let total_size = if *orientation == gtk4::Orientation::Horizontal {
                    paned.width()
                } else {
                    paned.height()
                };
                let ratio = if total_size > 0 {
                    (paned.position() as f64) / (total_size as f64)
                } else {
                    0.5 // default if not yet laid out
                };
                SplitNodeData::Split {
                    orientation: match orientation {
                        gtk4::Orientation::Horizontal => "horizontal".to_string(),
                        gtk4::Orientation::Vertical => "vertical".to_string(),
                        _ => "horizontal".to_string(),
                    },
                    ratio,
                    start: Box::new(start.to_data()),
                    end: Box::new(end.to_data()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_node_data_leaf_has_surface_uuid() {
        // Build a minimal SplitNodeData::Leaf directly and verify surface_uuid field exists.
        let id = Uuid::new_v4();
        let data = SplitNodeData::Leaf {
            pane_id: 42,
            surface_uuid: id,
            shell: "/bin/bash".to_string(),
            cwd: "/home/user".to_string(),
            agent_provider: None,
            agent_session_id: None,
        };
        if let SplitNodeData::Leaf {
            surface_uuid,
            pane_id,
            ..
        } = data
        {
            assert_eq!(surface_uuid, id);
            assert_eq!(pane_id, 42);
        } else {
            panic!("Expected SplitNodeData::Leaf");
        }
    }

    #[test]
    fn split_node_data_roundtrip_json() {
        // Verify SplitNodeData serializes and deserializes via serde_json.
        let leaf = SplitNodeData::Leaf {
            pane_id: 1,
            surface_uuid: Uuid::new_v4(),
            shell: "/bin/zsh".to_string(),
            cwd: "/tmp".to_string(),
            agent_provider: None,
            agent_session_id: None,
        };
        let json = serde_json::to_string(&leaf).expect("serialize failed");
        let restored: SplitNodeData = serde_json::from_str(&json).expect("deserialize failed");
        if let (
            SplitNodeData::Leaf {
                pane_id: p1,
                surface_uuid: u1,
                ..
            },
            SplitNodeData::Leaf {
                pane_id: p2,
                surface_uuid: u2,
                ..
            },
        ) = (&leaf, &restored)
        {
            assert_eq!(p1, p2);
            assert_eq!(u1, u2);
        } else {
            panic!("Roundtrip changed variant");
        }
    }

    #[test]
    fn split_node_data_split_roundtrip_json() {
        // Verify nested SplitNodeData serializes correctly with ratio field.
        let split = SplitNodeData::Split {
            orientation: "horizontal".to_string(),
            ratio: 0.35,
            start: Box::new(SplitNodeData::Leaf {
                pane_id: 1,
                surface_uuid: Uuid::new_v4(),
                shell: String::new(),
                cwd: String::new(),
                agent_provider: None,
                agent_session_id: None,
            }),
            end: Box::new(SplitNodeData::Leaf {
                pane_id: 2,
                surface_uuid: Uuid::new_v4(),
                shell: String::new(),
                cwd: String::new(),
                agent_provider: None,
                agent_session_id: None,
            }),
        };
        let json = serde_json::to_string(&split).expect("serialize failed");
        let restored: SplitNodeData = serde_json::from_str(&json).expect("deserialize failed");
        if let SplitNodeData::Split {
            orientation, ratio, ..
        } = restored
        {
            assert_eq!(orientation, "horizontal");
            assert!(
                (ratio - 0.35).abs() < f64::EPSILON,
                "ratio not preserved in roundtrip"
            );
        } else {
            panic!("Roundtrip changed variant to non-Split");
        }

        // Verify v1-compat: Split without ratio field deserializes with default 0.5
        let v1_json = r#"{"type":"Split","orientation":"vertical","start":{"type":"Leaf","pane_id":1,"surface_uuid":"00000000-0000-0000-0000-000000000000","shell":"","cwd":""},"end":{"type":"Leaf","pane_id":2,"surface_uuid":"00000000-0000-0000-0000-000000000000","shell":"","cwd":""}}"#;
        let v1_restored: SplitNodeData =
            serde_json::from_str(v1_json).expect("v1 deserialize failed");
        if let SplitNodeData::Split { ratio, .. } = v1_restored {
            assert!(
                (ratio - 0.5).abs() < f64::EPSILON,
                "v1 missing ratio should default to 0.5"
            );
        } else {
            panic!("v1 deserialize changed variant");
        }
    }
}

#[cfg(test)]
mod data_tests {
    use super::*;

    fn leaf(pane: u64) -> SplitNodeData {
        SplitNodeData::Leaf {
            pane_id: pane,
            surface_uuid: Uuid::new_v4(),
            shell: "/bin/bash".into(),
            cwd: "/tmp".into(),
            agent_provider: None,
            agent_session_id: None,
        }
    }

    #[test]
    fn leaf_serde_roundtrip() {
        let node = leaf(42);
        let json = serde_json::to_string(&node).expect("serialize");
        let back: SplitNodeData = serde_json::from_str(&json).expect("deserialize");
        match back {
            SplitNodeData::Leaf { pane_id, shell, .. } => {
                assert_eq!(pane_id, 42);
                assert_eq!(shell, "/bin/bash");
            }
            _ => panic!("expected leaf"),
        }
    }

    #[test]
    fn split_serde_roundtrip_preserves_ratio() {
        let node = SplitNodeData::Split {
            orientation: "horizontal".into(),
            ratio: 0.25,
            start: Box::new(leaf(1)),
            end: Box::new(leaf(2)),
        };
        let json = serde_json::to_string(&node).expect("serialize");
        let back: SplitNodeData = serde_json::from_str(&json).expect("deserialize");
        match back {
            SplitNodeData::Split {
                ratio, orientation, ..
            } => {
                assert!((ratio - 0.25).abs() < 1e-9);
                assert_eq!(orientation, "horizontal");
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn ratio_defaults_when_absent() {
        // Old session files predate the ratio field: strip it and reload.
        let node = SplitNodeData::Split {
            orientation: "vertical".into(),
            ratio: 0.9,
            start: Box::new(leaf(1)),
            end: Box::new(leaf(2)),
        };
        let mut v: serde_json::Value = serde_json::to_value(&node).expect("to_value");
        // Remove `ratio` wherever the tagging scheme put it.
        let obj = v.as_object_mut().expect("object");
        if !obj.contains_key("ratio") {
            for (_, inner) in obj.iter_mut() {
                if let Some(io) = inner.as_object_mut() {
                    io.remove("ratio");
                }
            }
        } else {
            obj.remove("ratio");
        }
        let back: SplitNodeData = serde_json::from_value(v).expect("deserialize");
        match back {
            SplitNodeData::Split { ratio, .. } => assert!((ratio - 0.5).abs() < 1e-9),
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn agent_fields_skip_when_none_survive_when_set() {
        let json = serde_json::to_string(&leaf(1)).expect("ser");
        assert!(
            !json.contains("agent_provider"),
            "None must be omitted: {json}"
        );

        let node = SplitNodeData::Leaf {
            pane_id: 2,
            surface_uuid: Uuid::new_v4(),
            shell: String::new(),
            cwd: String::new(),
            agent_provider: Some("claude".into()),
            agent_session_id: Some("sid".into()),
        };
        let json = serde_json::to_string(&node).expect("ser");
        assert!(json.contains("\"agent_provider\":\"claude\""));
        let back: SplitNodeData = serde_json::from_str(&json).expect("de");
        match back {
            SplitNodeData::Leaf {
                agent_session_id, ..
            } => {
                assert_eq!(agent_session_id.as_deref(), Some("sid"));
            }
            _ => panic!("expected leaf"),
        }
    }

    #[test]
    fn nested_tree_roundtrip() {
        let node = SplitNodeData::Split {
            orientation: "horizontal".into(),
            ratio: 0.5,
            start: Box::new(SplitNodeData::Split {
                orientation: "vertical".into(),
                ratio: 0.5,
                start: Box::new(leaf(1)),
                end: Box::new(leaf(2)),
            }),
            end: Box::new(leaf(3)),
        };
        let json = serde_json::to_string(&node).expect("ser");
        let back: SplitNodeData = serde_json::from_str(&json).expect("de");
        let mut ids = Vec::new();
        fn collect(n: &SplitNodeData, out: &mut Vec<u64>) {
            match n {
                SplitNodeData::Leaf { pane_id, .. } => out.push(*pane_id),
                SplitNodeData::Split { start, end, .. } => {
                    collect(start, out);
                    collect(end, out);
                }
            }
        }
        collect(&back, &mut ids);
        assert_eq!(ids, vec![1, 2, 3]);
    }
}
