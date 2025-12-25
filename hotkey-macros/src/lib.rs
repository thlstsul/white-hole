use proc_macro::TokenStream;
use quote::quote;
use syn::Token;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{ExprTuple, ItemFn, parse_macro_input, punctuated::Punctuated};

// 定义热键参数结构 - 支持单个热键或多个热键
enum HotkeyArgs {
    Single {
        modifiers: syn::Expr,
        _comma: Token![,],
        code: syn::Expr,
    },
    Multiple {
        hotkeys: Punctuated<ExprTuple, Token![,]>,
    },
}

impl Parse for HotkeyArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // 尝试解析为数组（多个热键）
        if input.peek(syn::token::Bracket) {
            let content;
            syn::bracketed!(content in input);

            let hotkeys = Punctuated::<ExprTuple, Token![,]>::parse_terminated(&content)?;

            if hotkeys.is_empty() {
                return Err(syn::Error::new(content.span(), "至少需要一个热键参数"));
            }

            Ok(HotkeyArgs::Multiple { hotkeys })
        } else {
            // 解析为单个热键
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
    // 解析输入函数
    let input_fn = parse_macro_input!(input as ItemFn);

    // 获取原函数信息
    let vis = &input_fn.vis;
    let fn_async = input_fn.sig.asyncness.is_some();
    let fn_name = &input_fn.sig.ident;
    let wrapper_name = syn::Ident::new(&format!("_{}", fn_name), fn_name.span());

    // 解析属性参数
    let args = parse_macro_input!(args as HotkeyArgs);

    // 生成注册代码
    let register_calls = match args {
        HotkeyArgs::Single {
            modifiers, code, ..
        } => {
            let call = if fn_async {
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
            };
            quote! { #call }
        }
        HotkeyArgs::Multiple { hotkeys } => {
            let mut calls = Vec::new();

            // 如果是异步函数，需要生成包装函数
            if fn_async {
                calls.push(quote! {
                    #vis fn #wrapper_name(app_handle: ::tauri::AppHandle) {
                        ::tauri::async_runtime::spawn(#fn_name(app_handle));
                    }
                });

                for (_i, hotkey_tuple) in hotkeys.iter().enumerate() {
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

                    calls.push(quote! {
                        manager.register(::hotkey::Hotkey::new(#modifiers, #code), #wrapper_name);
                    });
                }
            } else {
                for (_i, hotkey_tuple) in hotkeys.iter().enumerate() {
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

                    calls.push(quote! {
                        manager.register(::hotkey::Hotkey::new(#modifiers, #code), #fn_name);
                    });
                }
            }

            quote! { #(#calls)* }
        }
    };

    // 生成代码
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
