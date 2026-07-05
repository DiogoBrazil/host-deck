use leptos::prelude::*;

#[component]
pub fn ConfirmDialog(
    title: String,
    message: String,
    confirm_label: String,
    on_confirm: Callback<()>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="modal-backdrop" on:click=move |_| on_cancel.run(())>
            <div class="modal modal-sm" on:click=|ev| ev.stop_propagation()>
                <h2>{title}</h2>
                <p class="confirm-message">{message}</p>
                <div class="modal-actions">
                    <button class="btn" on:click=move |_| on_cancel.run(())>
                        "Cancelar"
                    </button>
                    <button class="btn btn-danger" on:click=move |_| on_confirm.run(())>
                        {confirm_label}
                    </button>
                </div>
            </div>
        </div>
    }
}
