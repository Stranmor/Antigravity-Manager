//! Button component with variants

use leptos::prelude::*;

#[derive(Clone, Copy, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Danger,
    Ghost,
}

impl ButtonVariant {
    fn class(&self) -> &'static str {
        match self {
            ButtonVariant::Primary => "btn--primary",
            ButtonVariant::Secondary => "btn--secondary",
            ButtonVariant::Danger => "btn--danger",
            ButtonVariant::Ghost => "btn--ghost",
        }
    }
}

#[component]
pub fn Button(
    #[prop(into)] children: Children,
    #[prop(optional)] variant: ButtonVariant,
    #[prop(optional)] disabled: bool,
    #[prop(optional)] loading: bool,
    #[prop(optional, into)] class: String,
    #[prop(into)] on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button 
            class=format!("btn {} {} {}", 
                variant.class(), 
                if loading { "btn--loading" } else { "" },
                class
            )
            disabled=disabled || loading
            on:click=move |_| on_click.call(())
        >
            {move || if loading {
                view! { <span class="btn__spinner"></span> }.into_any()
            } else {
                children().into_any()
            }}
        </button>
    }
}
