use std::rc::Rc;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::models::{AppError, AuthMethod, ConnectionInput, FieldError, SshConnection};

/// Create/edit modal. Secrets are write-only and are never prefilled.
#[component]
pub fn ConnectionForm(
    editing: Option<SshConnection>,
    on_saved: Callback<(SshConnection, bool)>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let is_edit = editing.is_some();
    let editing_id = editing.as_ref().map(|c| c.id.clone());
    let has_saved_password = editing
        .as_ref()
        .is_some_and(|c| c.password_secret_key.is_some());

    let init = editing.clone();
    let name = RwSignal::new(init.as_ref().map(|c| c.name.clone()).unwrap_or_default());
    let host = RwSignal::new(init.as_ref().map(|c| c.host.clone()).unwrap_or_default());
    let port = RwSignal::new(
        init.as_ref()
            .map(|c| c.port.to_string())
            .unwrap_or_default(),
    );
    let username = RwSignal::new(
        init.as_ref()
            .map(|c| c.username.clone())
            .unwrap_or_default(),
    );
    let auth_method = RwSignal::new(
        init.as_ref()
            .map(|c| c.auth_method)
            .unwrap_or(AuthMethod::Password),
    );
    let identity_file = RwSignal::new(
        init.as_ref()
            .and_then(|c| c.identity_file.clone())
            .unwrap_or_default(),
    );
    let group_name = RwSignal::new(
        init.as_ref()
            .map(|c| c.group_name.clone())
            .unwrap_or_default(),
    );
    let notes = RwSignal::new(
        init.as_ref()
            .and_then(|c| c.notes.clone())
            .unwrap_or_default(),
    );
    let password = RwSignal::new(String::new());
    let passphrase = RwSignal::new(String::new());
    let save_passphrase = RwSignal::new(false);

    let field_errors = RwSignal::new(Vec::<FieldError>::new());
    let general_error = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);

    let error_for = move |field: &'static str| {
        field_errors
            .get()
            .into_iter()
            .find(|e| e.field == field)
            .map(|e| e.message)
    };

    let do_submit: Rc<dyn Fn(bool)> = Rc::new(move |connect: bool| {
        if saving.get() {
            return;
        }
        field_errors.set(Vec::new());
        general_error.set(None);

        let port_value = {
            let raw = port.get();
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                match trimmed.parse::<u16>() {
                    Ok(p) => Some(p),
                    Err(_) => {
                        field_errors.set(vec![FieldError {
                            field: "port".into(),
                            message: "Porta deve ser um número entre 1 e 65535.".into(),
                        }]);
                        return;
                    }
                }
            }
        };

        let input = ConnectionInput {
            name: name.get(),
            host: host.get(),
            port: port_value,
            username: username.get(),
            auth_method: auth_method.get(),
            identity_file: {
                let v = identity_file.get();
                (!v.trim().is_empty()).then(|| v.trim().to_string())
            },
            group_name: group_name.get(),
            notes: {
                let v = notes.get();
                (!v.trim().is_empty()).then(|| v.trim().to_string())
            },
            password: {
                let v = password.get();
                (!v.is_empty()).then_some(v)
            },
            passphrase: {
                let v = passphrase.get();
                (!v.is_empty()).then_some(v)
            },
            save_passphrase: save_passphrase.get(),
        };

        saving.set(true);
        let editing_id = editing_id.clone();
        spawn_local(async move {
            let result = match &editing_id {
                Some(id) => api::update_connection(id, &input).await,
                None => api::create_connection(&input).await,
            };
            saving.set(false);

            match result {
                Ok(conn) => {
                    password.set(String::new());
                    passphrase.set(String::new());
                    on_saved.run((conn, connect));
                }
                Err(AppError::Validation(errors)) => field_errors.set(errors),
                Err(err) => general_error.set(Some(err.message())),
            }
        });
    });
    let submit_save = do_submit.clone();
    let submit_save_connect = do_submit.clone();

    view! {
        <div class="modal-backdrop" on:click=move |_| on_cancel.run(())>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <h2>{if is_edit { "Editar conexão" } else { "Nova conexão" }}</h2>

                {move || {
                    general_error
                        .get()
                        .map(|msg| view! { <div class="form-error-banner">{msg}</div> })
                }}

                <div class="form-grid">
                    <label class="form-field">
                        <span>"Nome"</span>
                        <input
                            type="text"
                            placeholder="Se vazio: usuario@host"
                            prop:value=move || name.get()
                            on:input=move |ev| name.set(event_target_value(&ev))
                        />
                    </label>

                    <label class="form-field">
                        <span>"Grupo"</span>
                        <input
                            type="text"
                            placeholder="Geral"
                            prop:value=move || group_name.get()
                            on:input=move |ev| group_name.set(event_target_value(&ev))
                        />
                    </label>

                    <label class="form-field span-2">
                        <span>"Host *"</span>
                        <input
                            type="text"
                            placeholder="ex.: 93.127.129.95 ou vps.exemplo.com"
                            prop:value=move || host.get()
                            on:input=move |ev| host.set(event_target_value(&ev))
                        />
                        {move || {
                            error_for("host").map(|m| view! { <span class="field-error">{m}</span> })
                        }}
                    </label>

                    <label class="form-field">
                        <span>"Usuário *"</span>
                        <input
                            type="text"
                            placeholder="ex.: root, ubuntu"
                            prop:value=move || username.get()
                            on:input=move |ev| username.set(event_target_value(&ev))
                        />
                        {move || {
                            error_for("username")
                                .map(|m| view! { <span class="field-error">{m}</span> })
                        }}
                    </label>

                    <label class="form-field">
                        <span>"Porta"</span>
                        <input
                            type="text"
                            placeholder="22"
                            prop:value=move || port.get()
                            on:input=move |ev| port.set(event_target_value(&ev))
                        />
                        {move || {
                            error_for("port").map(|m| view! { <span class="field-error">{m}</span> })
                        }}
                    </label>

                    <label class="form-field span-2">
                        <span>"Método de autenticação *"</span>
                        <select on:change=move |ev| {
                            auth_method
                                .set(
                                    if event_target_value(&ev) == "private_key" {
                                        AuthMethod::PrivateKey
                                    } else {
                                        AuthMethod::Password
                                    },
                                )
                        }>
                            <option
                                value="password"
                                selected=move || auth_method.get() == AuthMethod::Password
                            >
                                "Senha"
                            </option>
                            <option
                                value="private_key"
                                selected=move || auth_method.get() == AuthMethod::PrivateKey
                            >
                                "Chave SSH"
                            </option>
                        </select>
                    </label>

                    <Show when=move || auth_method.get() == AuthMethod::Password>
                        <label class="form-field span-2">
                            <span>{if has_saved_password {
                                "Senha (salva — preencha para alterar)"
                            } else {
                                "Senha *"
                            }}</span>
                            <input
                                type="password"
                                autocomplete="off"
                                prop:value=move || password.get()
                                on:input=move |ev| password.set(event_target_value(&ev))
                            />
                            {move || {
                                error_for("password")
                                    .map(|m| view! { <span class="field-error">{m}</span> })
                            }}
                        </label>
                    </Show>

                    <Show when=move || auth_method.get() == AuthMethod::PrivateKey>
                        <label class="form-field span-2">
                            <span>"Caminho da chave privada *"</span>
                            <input
                                type="text"
                                placeholder="ex.: /home/usuario/.ssh/id_ed25519"
                                prop:value=move || identity_file.get()
                                on:input=move |ev| identity_file.set(event_target_value(&ev))
                            />
                            {move || {
                                error_for("identity_file")
                                    .map(|m| view! { <span class="field-error">{m}</span> })
                            }}
                        </label>
                        <label class="form-field span-2">
                            <span>"Passphrase da chave (opcional)"</span>
                            <input
                                type="password"
                                autocomplete="off"
                                prop:value=move || passphrase.get()
                                on:input=move |ev| passphrase.set(event_target_value(&ev))
                            />
                        </label>
                        <label class="form-check span-2">
                            <input
                                type="checkbox"
                                prop:checked=move || save_passphrase.get()
                                on:change=move |ev| {
                                    save_passphrase.set(event_target_checked(&ev))
                                }
                            />
                            <span>"Salvar passphrase no armazenamento seguro do sistema"</span>
                        </label>
                    </Show>

                    <label class="form-field span-2">
                        <span>"Observações"</span>
                        <textarea
                            rows="3"
                            prop:value=move || notes.get()
                            on:input=move |ev| notes.set(event_target_value(&ev))
                        ></textarea>
                    </label>
                </div>

                <div class="modal-actions">
                    <button class="btn" on:click=move |_| on_cancel.run(())>
                        "Cancelar"
                    </button>
                    <button
                        class="btn"
                        disabled=move || saving.get()
                        on:click=move |_| submit_save(false)
                    >
                        {move || if saving.get() { "Salvando…" } else { "Salvar" }}
                    </button>
                    <button
                        class="btn btn-primary"
                        disabled=move || saving.get()
                        on:click=move |_| submit_save_connect(true)
                    >
                        "Salvar e conectar"
                    </button>
                </div>
            </div>
        </div>
    }
}
