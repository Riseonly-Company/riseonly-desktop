use rise_ui::swiftui::binding::Binding;

pub trait BindingStableElement<Element> {
    fn stable_element<Id>(&self, item: Element, id_of: impl Fn(&Element) -> Id + 'static) -> Binding<Element>
    where
        Id: PartialEq + 'static;
}

impl<Element: Clone + 'static> BindingStableElement<Element> for Binding<Vec<Element>> {
    fn stable_element<Id>(&self, item: Element, id_of: impl Fn(&Element) -> Id + 'static) -> Binding<Element>
    where
        Id: PartialEq + 'static,
    {
        let id = id_of(&item);
        let id_of = std::rc::Rc::new(id_of);
        let id = std::rc::Rc::new(id);
        let reader = self.clone();
        let writer = self.clone();
        let read_id = (id.clone(), id_of.clone());
        Binding::new(
            move |cx| {
                let (id, id_of) = (&read_id.0, &read_id.1);
                reader
                    .get(cx)
                    .into_iter()
                    .find(|element| id_of(element) == **id)
                    .unwrap_or_else(|| item.clone())
            },
            move |new_value, window, cx| {
                let mut elements = writer.get(cx);
                let Some(index) = elements.iter().position(|element| id_of(element) == *id) else {
                    return;
                };
                elements[index] = new_value;
                writer.set(elements, window, cx);
            },
        )
    }
}
