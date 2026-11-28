//! Canvas over membership: `page_document` keeps unassigned on-page shapes with the page
//! offset applied, `render_thumbnail` draws exactly the output operations' members, and
//! `shape_locked` honours locked layers and operations.

use craft_stencilcraft_document::{Document, Layer, Operation, OperationKind, OperationSettings, Page, PageSetup};
use kurbo::BezPath;

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h));
    p.line_to((x, y + h));
    p.close_path();
    p
}

fn op(id: u64, name: &str, kind: OperationKind, locked: bool, members: Vec<u64>) -> Operation {
    Operation { id, name: name.to_string(), kind, settings: OperationSettings::default(), color: None, visible: true, locked, members, icon: None }
}

/// Layers: 0 cut, 1 engrave, 2 guide, 3 locked. Operations: 0 Cut (member 1),
/// 1 Engrave (member 1 — shared), 2 Guide (member 2), 3 locked Cut (member 3).
/// Shape 4 sits on page 1 and belongs to no operation.
fn fixture() -> Result<Document, Box<dyn std::error::Error>> {
    let mut doc = Document {
        layers: vec![Layer::new("cut"), Layer::new("engrave"), Layer::new("guide"), Layer { locked: true, ..Layer::new("locked-cut") }],
        ..Document::default()
    };
    doc.operations = vec![
        op(1, "cut", OperationKind::Cut, false, vec![1]),
        op(2, "engrave", OperationKind::Engrave, false, vec![1]),
        op(3, "guide", OperationKind::Guide, false, vec![2]),
        op(4, "locked-cut", OperationKind::Cut, true, vec![3]),
    ];
    let a = doc.push_shape(0, rect_path(5.0, 5.0, 10.0, 10.0))?;
    let b = doc.push_shape(2, rect_path(30.0, 5.0, 10.0, 10.0))?;
    let c = doc.push_shape(3, rect_path(50.0, 5.0, 10.0, 10.0))?;
    doc.pages.push(Page {
        x_mm: 120.0,
        y_mm: 0.0,
        width_mm: 210.0,
        height_mm: 297.0,
        setup: PageSetup { name: "A4".to_string(), ..PageSetup::default() },
    });
    doc.shapes.push(craft_stencilcraft_document::Shape {
        id: 4,
        path: rect_path(130.0, 10.0, 10.0, 10.0),
        hidden: false,
        locked: false,
        text: None,
        production: Default::default(),
    });
    doc.next_shape_id = 5;
    let _ = (a.id, b.id, c.id);
    Ok(doc)
}

#[test]
fn page_document_keeps_the_unassigned_on_page_shape_with_offset() -> Result<(), Box<dyn std::error::Error>> {
    let doc = fixture()?;
    let page = doc.page_document(1).expect("page 1 exists");
    assert_eq!((page.board.width_mm, page.board.height_mm), (210.0, 297.0));
    assert_eq!(page.shape_count(), 1, "the unassigned on-page shape is never dropped");
    assert!(page.unassigned_shapes().all(|s| s.id == 4), "it stays unassigned on its own page");
    let b = page.shapes[0].bounds().expect("bounds");
    assert!((b.x0 - 10.0).abs() < 1e-9 && (b.y0 - 10.0).abs() < 1e-9, "geometry is shifted by the page offset");
    assert!(page.layers.iter().all(|l| l.members.is_empty()), "off-page layer memberships are pruned");
    assert!(page.operations.iter().all(|o| o.members.is_empty()), "off-page operation memberships are pruned");
    assert_eq!(doc.page_document(0).expect("page 0").shape_count(), 3, "page 0 keeps its three shapes");
    Ok(())
}

#[test]
fn render_thumbnail_reflects_operation_membership() -> Result<(), Box<dyn std::error::Error>> {
    let doc = fixture()?;
    let svg = doc.render_thumbnail(64 * 1024);
    // Shape 1 is a member of the Cut and the Engrave operation: two paths. The Guide
    // member (shape 2) produces nothing; the unassigned shape (4) is not drawn.
    assert_eq!(svg.matches("<path").count(), 3, "one path per output-operation membership: cut+engrave share shape 1, locked-cut draws shape 3");
    let mut hidden = doc.clone();
    hidden.operations[3].visible = false;
    assert_eq!(hidden.render_thumbnail(64 * 1024).matches("<path").count(), 2, "an invisible operation is not rendered");
    Ok(())
}

#[test]
fn shape_locked_honours_locked_layers_and_operations() -> Result<(), Box<dyn std::error::Error>> {
    let doc = fixture()?;
    assert!(doc.shape_locked(3), "a shape in a locked layer/operation is locked");
    assert!(!doc.shape_locked(1), "a shape in unlocked layers is not locked");
    assert!(!doc.shape_locked(999), "an unknown shape is not locked");
    Ok(())
}
