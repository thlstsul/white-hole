use proc_macro::TokenStream;
use quote::quote;
use syn::Token;
use syn::parse::{Parse, ParseStream};
use syn::{ItemFn, parse_macro_input};

// 定义热键参数结构
struct HotkeyArgs {
    modifiers: syn::Expr,
    _comma: Token![,],
    code: syn::Expr,
}

impl Parse for HotkeyArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(HotkeyArgs {
            modifiers: input.parse()?,
            _comma: input.parse()?,
            code: input.parse()?,
        })
    }
}

#[proc_macro_attribute]
pub fn hotkey(args: TokenStream, input: TokenStream) -> TokenStream {
    // 解析输入函数
    let input_fn = parse_macro_input!(input as ItemFn);

    // 获取原函数信息
    let vis = &input_fn.vis;
    let fn_name = &input_fn.sig.ident;
    let wrapper_name = syn::Ident::new(&format!("_{}", fn_name), fn_name.span());

    // 解析属性参数
    let args = parse_macro_input!(args as HotkeyArgs);

    let modifiers = &args.modifiers;
    let code = &args.code;

    // 生成代码
    let expanded = quote! {
        #input_fn

        #[allow(non_camel_case_types, missing_docs)]
        #vis struct #wrapper_name;

        impl ::hotkey::HotkeyRegistrar for #wrapper_name {
            fn register(&self, mut manager: ::hotkey::HotkeyManager<::tauri::Wry>) -> ::hotkey::HotkeyManager<::tauri::Wry> {
                #vis fn #wrapper_name(app_handle: &::tauri::AppHandle) {
                    ::tauri::async_runtime::spawn(#fn_name(app_handle.clone()));
                }
                manager.register(::hotkey::Hotkey::new(#modifiers, #code), #wrapper_name);
                manager
            }
        }

        ::hotkey::submit_hotkey!(#wrapper_name);
    };

    TokenStream::from(expanded)
}
