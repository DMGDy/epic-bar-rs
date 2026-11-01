/* A status value that keeps the width of its widest usual reading, measured
 * with the label's own font, so updates do not shift neighbouring modules. */
use gtk::{glib, prelude::*, subclass::prelude::*, Label, Orientation};

mod imp {
    use std::cell::{OnceCell, RefCell};

    use super::*;

    #[derive(Default)]
    pub struct Slot {
        pub label: OnceCell<Label>,
        pub samples: RefCell<Vec<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Slot {
        const NAME: &'static str = "EpicBarMetricSlot";
        type Type = super::Slot;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("metric-slot");
        }
    }

    impl ObjectImpl for Slot {
        fn dispose(&self) {
            if let Some(label) = self.label.get() {
                label.unparent();
            }
        }
    }

    impl WidgetImpl for Slot {
        fn measure(&self, orientation: Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let label = self.label.get().unwrap();
            let (minimum, natural, baseline_min, baseline_nat) =
                label.measure(orientation, for_size);
            if orientation != Orientation::Horizontal {
                return (minimum, natural, baseline_min, baseline_nat);
            }
            // A copy of the label's own layout carries its font, tabular
            // digits and any CSS text attributes.
            let layout = label.layout().copy();
            layout.set_width(-1);
            layout.set_ellipsize(gtk::pango::EllipsizeMode::None);
            let text_width = |text: &str| {
                layout.set_text(text);
                layout.pixel_size().0
            };
            // The label's CSS margins, borders and padding around its text.
            let chrome = natural - text_width(&label.text());
            let reserved = self
                .samples
                .borrow()
                .iter()
                .map(|sample| text_width(sample) + chrome)
                .max()
                .unwrap_or(0);
            // Longer values than the samples still grow instead of clipping.
            (
                minimum.max(reserved),
                natural.max(reserved),
                baseline_min,
                baseline_nat,
            )
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.label
                .get()
                .unwrap()
                .allocate(width, height, baseline, None);
        }
    }
}

glib::wrapper! {
    pub struct Slot(ObjectSubclass<imp::Slot>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Slot {
    /* `samples` are the widest values `label` normally shows. */
    pub fn new(label: &Label, samples: &[&str]) -> Self {
        let slot: Self = glib::Object::new();
        let imp = slot.imp();
        imp.samples
            .replace(samples.iter().map(ToString::to_string).collect());
        label.set_parent(&slot);
        imp.label.set(label.clone()).unwrap();
        // Centring keeps short values visibly closer to their own icon than
        // to the next module's.
        label.set_xalign(0.5);
        // Hiding the value hides its reserved space too.
        label
            .bind_property("visible", &slot, "visible")
            .sync_create()
            .build();
        slot
    }

    pub fn label(&self) -> Label {
        self.imp().label.get().unwrap().clone()
    }
}
