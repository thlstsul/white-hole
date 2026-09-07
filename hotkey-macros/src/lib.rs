use proc_macro::TokenStream;
use quote::quote;
use syn::Token;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{ExprTuple, ItemFn, parse_macro_input, punctuated::Punctuated};

enum HotkeyArgs {
    Single {
        modifiers: Box<syn::Expr>,
        _comma: Token![,],
        code: Box<syn::Expr>,
    },
    Multiple {
        hotkeys: Punctuated<ExprTuple, Token![,]>,
    },
}

impl Parse for HotkeyArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(syn::token::Bracket) {
            let content;
            syn::bracketed!(content in input);

            let hotkeys = Punctuated::<ExprTuple, Token![,]>::parse_terminated(&content)?;

            if hotkeys.is_empty() {
                return Err(syn::Error::new(content.span(), "至少需要一个热键参数"));
            }

            Ok(HotkeyArgs::Multiple { hotkeys })
        } else {
            Ok(HotkeyArgs::Single {
                modifiers: input.parse()?,
                _comma: input.parse()?,
                code: input.parse()?,
            })
        }
    }
}

#[proc_macro_attribute]
pub fn hotkey(args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);

    let vis = &input_fn.vis;
    let fn_async = input_fn.sig.asyncness.is_some();
    let fn_name = &input_fn.sig.ident;
    let wrapper_name = syn::Ident::new(&format!("_{}", fn_name), fn_name.span());

    let args = parse_macro_input!(args as HotkeyArgs);

    let register_calls = match args {
        HotkeyArgs::Single {
            modifiers, code, ..
        } => {
            if fn_async {
                quote! {
                    #vis fn #wrapper_name(app_handle: ::tauri::AppHandle) {
                        ::tauri::async_runtime::spawn(#fn_name(app_handle));
                    }
                    manager.register(::hotkey::Hotkey::new(#modifiers, #code), #wrapper_name);
                }
            } else {
                quote! {
                    manager.register(::hotkey::Hotkey::new(#modifiers, #code), #fn_name);
                }
            }
        }
        HotkeyArgs::Multiple { hotkeys } => {
            let mut calls = Vec::new();

            if fn_async {
                calls.push(quote! {
                    #vis fn #wrapper_name(app_handle: ::tauri::AppHandle) {
                        ::tauri::async_runtime::spawn(#fn_name(app_handle));
                    }
                });
            }

            for hotkey_tuple in hotkeys.iter() {
                if hotkey_tuple.elems.len() != 2 {
                    return syn::Error::new(
                        hotkey_tuple.span(),
                        "每个热键参数必须是 (modifiers, code) 形式的元组",
                    )
                    .to_compile_error()
                    .into();
                }

                let modifiers = &hotkey_tuple.elems[0];
                let code = &hotkey_tuple.elems[1];

                if fn_async {
                    calls.push(quote! {
                        manager.register(::hotkey::Hotkey::new(#modifiers, #code), #wrapper_name);
                    });
                } else {
                    calls.push(quote! {
                        manager.register(::hotkey::Hotkey::new(#modifiers, #code), #fn_name);
                    });
                }
            }

            quote! { #(#calls)* }
        }
    };

    let expanded = quote! {
        #input_fn

        #[allow(non_camel_case_types, missing_docs)]
        #vis struct #wrapper_name;

        impl ::hotkey::HotkeyRegistrar for #wrapper_name {
            fn register(&self, mut manager: ::hotkey::HotkeyManager<::tauri::Wry>) -> ::hotkey::HotkeyManager<::tauri::Wry> {
                #register_calls
                manager
            }
        }

        ::hotkey::submit_hotkey!(#wrapper_name);
    };

    TokenStream::from(expanded)
}
