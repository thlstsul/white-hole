use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

#[proc_macro_attribute]
pub fn hotkey(_args: TokenStream, input: TokenStream) -> TokenStream {
    // 解析输入的函数
    let input_fn = parse_macro_input!(input as ItemFn);

    // 获取函数名、参数、函数体等信息
    let fn_name = &input_fn.sig.ident;
    let fn_block = &input_fn.block;
    let fn_attrs = &input_fn.attrs;
    let fn_vis = &input_fn.vis;

    // 检查函数签名是否符合要求
    let sig = &input_fn.sig;
    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(sig, "hotkey macro can only be applied to async functions")
            .to_compile_error()
            .into();
    }

    // 提取参数信息 - 假设只有一个参数：app_handle: AppHandle
    let inputs = &sig.inputs;
    if inputs.len() != 1 {
        return syn::Error::new_spanned(
            inputs,
            "hotkey function must have exactly one parameter: app_handle: AppHandle",
        )
        .to_compile_error()
        .into();
    }

    // 生成输出代码
    let expanded = quote! {
        #(#fn_attrs)*
        #fn_vis fn #fn_name(app_handle: &AppHandle) {
            async fn #fn_name(app_handle: AppHandle) #fn_block

            async_runtime::spawn(#fn_name(app_handle.clone()));
        }
    };

    expanded.into()
}
