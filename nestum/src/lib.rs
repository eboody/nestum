//! `nestum` lets real nested enum trees read like their shape.
//!
//! If your crate already has error envelopes, command trees, or event and message
//! trees, `nestum` keeps the same nested-enum model and removes most of the
//! wrapping noise around constructors and pattern matches.
//!
//! ```rust,ignore
//! use nestum::{nestum, nested};
//!
//! #[nestum]
//! enum DocumentEvent {
//!     Created,
//!     Deleted,
//! }
//!
//! #[nestum]
//! enum Event {
//!     Document(DocumentEvent),
//! }
//!
//! let event: Event::Enum = Event::Document::Created;
//!
//! nested! {
//!     match event {
//!         Event::Document::Created => {}
//!         Event::Document::Deleted => {}
//!     }
//! }
//! ```
//!
//! `nestum` is worth it when the outer enum is already a real envelope over
//! command, event, message, or error families and the main pain is wrapper
//! syntax, not the model itself.
//!
//! Main entry points:
//!
//! - `#[nestum]` marks an enum so nested enum-wrapping variants can be
//!   constructed through path-shaped syntax.
//! - `nested! { ... }` rewrites nested constructors and nested patterns for
//!   `match`, `if let`, `while let`, `let-else`, `matches!`, and common
//!   assertion macros.
//! - `#[nestum_scope]` rewrites a wider function, impl, method, or inline module
//!   body when local `nested!` wrappers would get noisy.
//!
//! Authority surface:
//!
//! - `nestum` resolves nesting from parsed crate-local source plus proc-macro
//!   source locations.
//! - `#[cfg]`, `#[cfg_attr]`, `include!()`, `#[path = "..."]`, macro-generated
//!   local enums, and external crates are outside that surface.
//! - unsupported cases are rejected where `nestum` can detect them, and it
//!   errors instead of guessing when source context is unavailable.
//!
//! For the migration guide, ecosystem cookbooks, and full examples, see the
//! repository README and the `nestum-examples` workspace crate.

use std::collections::{HashMap, HashSet};

use darling::FromMeta;
use darling::ast::NestedMeta;
use module_path_extractor::{
    module_path_from_file_with_root, module_path_to_file, module_root_from_file,
};
use proc_macro::TokenStream;
use proc_macro_error2::proc_macro_error;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ExprBlock, ExprCall, ExprLet, ExprMacro, ExprMatch, ExprPath, ExprStruct,
    Fields, ImplItemFn, Item, ItemEnum, ItemMod, Local, Meta, Pat, PatOr, PatPath, PatStruct,
    PatTupleStruct, Stmt, Token, TraitItemFn,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    spanned::Spanned,
    visit_mut::{self, VisitMut},
};

type EnumMap = HashMap<String, ItemEnum>;
type ModuleEnumCache = HashMap<String, EnumMap>;

#[derive(Clone, Copy)]
enum ModulePathBase {
    Current,
    Crate,
    Super(usize),
}

#[derive(Clone)]
struct PlainEnumPath {
    path_base: ModulePathBase,
    module_path: Vec<syn::Ident>,
    enum_ident: syn::Ident,
    enum_arguments: syn::PathArguments,
}

struct ResolveCtx<'a> {
    current_file: &'a str,
    module_root: &'a std::path::Path,
    current_module: &'a str,
    enums_by_ident: &'a EnumMap,
    cache: &'a mut ModuleEnumCache,
}

struct PathResolveCtx<'a> {
    current_file: &'a str,
    module_root: &'a std::path::Path,
    current_module: &'a str,
    enums_by_ident: &'a EnumMap,
}

#[derive(Clone)]
struct ResolvedEnumInfo {
    item: ItemEnum,
    marked: bool,
    module_path_str: String,
    base_path: syn::Path,
    enum_type_path: syn::Path,
}

#[derive(Clone)]
struct ResolvedVariantStep {
    owner: ResolvedEnumInfo,
    variant: syn::Variant,
}

struct ResolvedMatchPath {
    steps: Vec<ResolvedVariantStep>,
}

#[proc_macro_error]
#[proc_macro_attribute]
pub fn nestum(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as Item);
    match item {
        Item::Enum(item_enum) => {
            if !args.is_empty() {
                return syn::Error::new(
                    item_enum.ident.span(),
                    format!(
                        "invalid #[nestum(...)] on enum {}; \
nestum does not accept arguments. Use #[nestum] on enums only",
                        item_enum.ident
                    ),
                )
                .to_compile_error()
                .into();
            }
            expand_enum(item_enum)
                .unwrap_or_else(|err| err.to_compile_error())
                .into()
        }
        other => syn::Error::new(other.span(), "nestum can only be applied to enums")
            .to_compile_error()
            .into(),
    }
}

#[proc_macro_error]
#[proc_macro]
pub fn nestum_match(input: TokenStream) -> TokenStream {
    let input = proc_macro2::TokenStream::from(input);
    let expr = match syn::parse2::<ExprMatch>(input) {
        Ok(expr) => expr,
        Err(err) => {
            return syn::Error::new(
                err.span(),
                format!(
                    "nestum_match! expects a `match` expression; \
use nested! or #[nestum_scope] for constructors, if let, while let, let-else, and matches!: {err}"
                ),
            )
            .to_compile_error()
            .into();
        }
    };
    expand_match(expr)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

#[proc_macro_error]
#[proc_macro]
pub fn nested(input: TokenStream) -> TokenStream {
    parse_nested_input(proc_macro2::TokenStream::from(input))
        .and_then(expand_nested_input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

#[proc_macro_error]
#[proc_macro_attribute]
pub fn nestum_scope(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "nestum_scope does not accept arguments; use #[nestum_scope] on functions, impl methods, impl blocks, or inline modules",
        )
        .to_compile_error()
        .into();
    }

    let input = proc_macro2::TokenStream::from(input);

    if let Ok(mut item) = syn::parse2::<Item>(input.clone()) {
        return rewrite_scope_item_tokens(&mut item)
            .unwrap_or_else(|err| err.to_compile_error())
            .into();
    }

    if let Ok(mut item_fn) = syn::parse2::<ImplItemFn>(input.clone()) {
        return rewrite_scope_impl_item_fn_tokens(&mut item_fn)
            .unwrap_or_else(|err| err.to_compile_error())
            .into();
    }

    if let Ok(mut item_fn) = syn::parse2::<TraitItemFn>(input) {
        return rewrite_scope_trait_item_fn_tokens(&mut item_fn)
            .unwrap_or_else(|err| err.to_compile_error())
            .into();
    }

    syn::Error::new(
        proc_macro2::Span::call_site(),
        "#[nestum_scope] can only be applied to functions, impl methods, impl blocks, or inline modules",
    )
    .to_compile_error()
    .into()
}

enum NestedInput {
    Expr(Box<Expr>),
    Stmts(Vec<Stmt>),
}

fn parse_nested_input(tokens: proc_macro2::TokenStream) -> syn::Result<NestedInput> {
    if let Ok(expr) = syn::parse2::<Expr>(tokens.clone()) {
        return Ok(NestedInput::Expr(Box::new(expr)));
    }

    let wrapped = quote!({ #tokens });
    let block = syn::parse2::<ExprBlock>(wrapped).map_err(|err| {
        syn::Error::new(
            err.span(),
            format!(
                "nested! expects an expression or a statement block; \
for let-else, pass statements directly inside nested! {{ ... }}; \
for broader rewriting, use #[nestum_scope] on the enclosing function, impl, or inline module: {err}"
            ),
        )
    })?;
    Ok(NestedInput::Stmts(block.block.stmts))
}

fn expand_nested_input(input: NestedInput) -> Result<proc_macro2::TokenStream, syn::Error> {
    match input {
        NestedInput::Expr(expr) => expand_nested_expr(*expr),
        NestedInput::Stmts(stmts) => {
            let block = Expr::Block(ExprBlock {
                attrs: Vec::new(),
                label: None,
                block: syn::Block {
                    brace_token: Default::default(),
                    stmts,
                },
            });
            expand_nested_expr(block)
        }
    }
}

fn resolve_current_module_enums(
    cache: &ModuleEnumCache,
    requested_module_path: &str,
    file_path: &str,
    module_root: &std::path::Path,
) -> Option<(String, EnumMap)> {
    if let Some(enums) = cache.get(requested_module_path) {
        return Some((requested_module_path.to_string(), enums.clone()));
    }

    let fallback = module_path_from_file_with_root(file_path, module_root);
    if let Some(enums) = cache.get(&fallback) {
        return Some((fallback, enums.clone()));
    }

    if let Some(enums) = cache.get("crate") {
        return Some(("crate".to_string(), enums.clone()));
    }

    None
}

fn callsite_source_info() -> Option<(String, Option<usize>)> {
    if let Some((file_path, line_number)) = module_path_extractor::get_source_info() {
        return Some((
            absolutize_source_path(&file_path),
            if line_number == 0 {
                None
            } else {
                Some(line_number)
            },
        ));
    }

    let span = proc_macro2::Span::call_site();
    let file_path = resolve_span_file_path(&span)?;
    let line_number = span.start().line;
    Some((
        absolutize_source_path(&file_path),
        if line_number == 0 {
            None
        } else {
            Some(line_number)
        },
    ))
}

fn absolutize_source_path(file_path: &str) -> String {
    let path = std::path::Path::new(file_path);
    if path.is_absolute() {
        return normalize_file_key(file_path);
    }

    let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        return normalize_file_key(file_path);
    };
    let joined = std::path::Path::new(&manifest_dir).join(path);
    if joined.exists() {
        normalize_file_key(&joined.to_string_lossy())
    } else {
        normalize_file_key(file_path)
    }
}

fn strip_nestum_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|attr| !is_nestum_attr_path(attr.path()))
        .cloned()
        .collect()
}

fn cleaned_enum_item(item: &ItemEnum, variants: Vec<syn::Variant>) -> ItemEnum {
    let mut rewritten = item.clone();
    rewritten.attrs = strip_nestum_attrs(&item.attrs);
    rewritten.attrs.insert(0, syn::parse_quote!(#[doc(hidden)]));
    rewritten.variants = Punctuated::from_iter(variants);
    rewritten
}

fn ensure_reserved_generated_names_available(item: &ItemEnum) -> Result<(), syn::Error> {
    let reserved = generated_enum_alias_ident();
    if let Some(variant) = item
        .variants
        .iter()
        .find(|variant| variant.ident == reserved)
    {
        return Err(syn::Error::new(
            variant.ident.span(),
            format!(
                "variant {}::{} is not supported; `Enum` is reserved by #[nestum] for the explicit enum type path",
                item.ident, variant.ident
            ),
        ));
    }
    Ok(())
}

fn expand_enum(item: ItemEnum) -> Result<proc_macro2::TokenStream, syn::Error> {
    ensure_reserved_generated_names_available(&item)?;
    let (file_path, module_root, module_path) = current_module_context(item.span())?;
    ensure_supported_enum_shape(&item, item.span())?;
    let mut cache: ModuleEnumCache = HashMap::new();
    let all = collect_enums_by_module_path(&file_path, &module_root, &file_path)?;
    for (module, enums) in all.into_iter() {
        cache.insert(module, enums);
    }
    let (effective_module_path, enums_by_ident) =
        resolve_current_module_enums(&cache, &module_path, &file_path, &module_root).ok_or_else(
            || {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "no enums found for current module path; \
ensure the enum is defined in the same source file and module as the macro call",
                )
            },
        )?;

    let mut marked_enums = HashSet::new();
    for (name, info) in enums_by_ident.iter() {
        match nestum_attr_kind(&info.attrs)? {
            NestumAttrKind::None => {}
            NestumAttrKind::Empty => {
                marked_enums.insert(name.clone());
            }
            NestumAttrKind::WithArgs => {
                return Err(syn::Error::new(
                    info.span(),
                    format!(
                        "invalid #[nestum(...)] on enum {name}; \
nestum does not accept arguments. Use #[nestum] on enums only"
                    ),
                ));
            }
        }
    }

    let expanded = expand_enum_with_context(
        item,
        &enums_by_ident,
        &marked_enums,
        &effective_module_path,
        &file_path,
        &module_root,
        &mut cache,
    )?;
    Ok(expanded)
}

fn expand_match(expr: ExprMatch) -> Result<proc_macro2::TokenStream, syn::Error> {
    expand_nested_expr(Expr::Match(expr))
}

fn expand_nested_expr(expr: Expr) -> Result<proc_macro2::TokenStream, syn::Error> {
    let expr_span = expr.span();
    let (file_path, module_root, module_path) = current_module_context(expr_span)?;
    let mut cache: ModuleEnumCache = HashMap::new();
    let all = collect_enums_by_module_path(&file_path, &module_root, &file_path)?;
    for (module, enums) in all.into_iter() {
        cache.insert(module, enums);
    }
    let (effective_module_path, enums_by_ident) =
        resolve_current_module_enums(&cache, &module_path, &file_path, &module_root).ok_or_else(
            || {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "no enums found for current module path; \
ensure the enum is defined in the same source file and module as the macro call",
                )
            },
        )?;

    let mut expr = expr;
    let mut rewriter = NestedExprRewriter {
        current_file: &file_path,
        module_root: &module_root,
        current_module: &effective_module_path,
        enums_by_ident: &enums_by_ident,
        cache: &mut cache,
        error: None,
    };
    rewriter.visit_expr_mut(&mut expr);
    if let Some(err) = rewriter.error {
        return Err(err);
    }

    Ok(quote! { #expr })
}

fn collect_module_cache(
    file_path: &str,
    module_root: &std::path::Path,
) -> Result<ModuleEnumCache, syn::Error> {
    let mut cache: ModuleEnumCache = HashMap::new();
    let all = collect_enums_by_module_path(file_path, module_root, file_path)?;
    for (module, enums) in all.into_iter() {
        cache.insert(module, enums);
    }
    Ok(cache)
}

fn enums_for_exact_module(cache: &ModuleEnumCache, module_path: &str) -> EnumMap {
    cache.get(module_path).cloned().unwrap_or_default()
}

fn child_module_path(parent_module: &str, child_ident: &syn::Ident) -> String {
    if parent_module == "crate" {
        child_ident.to_string()
    } else {
        format!("{parent_module}::{child_ident}")
    }
}

fn with_scope_rewriter<F>(
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    cache: &mut ModuleEnumCache,
    apply: F,
) -> Result<(), syn::Error>
where
    F: FnOnce(&mut NestedExprRewriter<'_>),
{
    let enums_by_ident = enums_for_exact_module(cache, current_module);
    let mut rewriter = NestedExprRewriter {
        current_file,
        module_root,
        current_module,
        enums_by_ident: &enums_by_ident,
        cache,
        error: None,
    };
    apply(&mut rewriter);
    if let Some(err) = rewriter.error {
        return Err(err);
    }
    Ok(())
}

fn rewrite_scope_item_with_context(
    item: &mut Item,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    cache: &mut ModuleEnumCache,
) -> Result<(), syn::Error> {
    match item {
        Item::Mod(item_mod) => {
            rewrite_scope_inline_module(item_mod, current_file, module_root, current_module, cache)
        }
        _ => with_scope_rewriter(
            current_file,
            module_root,
            current_module,
            cache,
            |rewriter| {
                rewriter.visit_item_mut(item);
            },
        ),
    }
}

fn rewrite_scope_inline_module(
    item_mod: &mut ItemMod,
    current_file: &str,
    module_root: &std::path::Path,
    parent_module: &str,
    cache: &mut ModuleEnumCache,
) -> Result<(), syn::Error> {
    let child_module = child_module_path(parent_module, &item_mod.ident);
    let Some((_, items)) = &mut item_mod.content else {
        return Err(syn::Error::new(
            item_mod.span(),
            "#[nestum_scope] requires an inline module body; use it on `mod name { ... }`, not `mod name;`",
        ));
    };

    for item in items {
        rewrite_scope_item_with_context(item, current_file, module_root, &child_module, cache)?;
    }

    Ok(())
}

fn rewrite_scope_item_tokens(item: &mut Item) -> Result<proc_macro2::TokenStream, syn::Error> {
    let (current_file, module_root, current_module) = current_module_context(item.span())?;
    let mut cache = collect_module_cache(&current_file, &module_root)?;

    match item {
        Item::Fn(_) | Item::Impl(_) => {
            rewrite_scope_item_with_context(
                item,
                &current_file,
                &module_root,
                &current_module,
                &mut cache,
            )?;
        }
        Item::Mod(item_mod) => {
            rewrite_scope_inline_module(
                item_mod,
                &current_file,
                &module_root,
                &current_module,
                &mut cache,
            )?;
        }
        _ => {
            return Err(syn::Error::new(
                item.span(),
                "#[nestum_scope] can only be applied to functions, impl blocks, or inline modules",
            ));
        }
    }

    Ok(quote! { #item })
}

fn rewrite_scope_impl_item_fn_tokens(
    item_fn: &mut ImplItemFn,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let (current_file, module_root, current_module) = current_module_context(item_fn.span())?;
    let mut cache = collect_module_cache(&current_file, &module_root)?;
    with_scope_rewriter(
        &current_file,
        &module_root,
        &current_module,
        &mut cache,
        |rewriter| {
            rewriter.visit_impl_item_fn_mut(item_fn);
        },
    )?;
    Ok(quote! { #item_fn })
}

fn rewrite_scope_trait_item_fn_tokens(
    item_fn: &mut TraitItemFn,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let (current_file, module_root, current_module) = current_module_context(item_fn.span())?;
    let mut cache = collect_module_cache(&current_file, &module_root)?;
    with_scope_rewriter(
        &current_file,
        &module_root,
        &current_module,
        &mut cache,
        |rewriter| {
            rewriter.visit_trait_item_fn_mut(item_fn);
        },
    )?;
    Ok(quote! { #item_fn })
}
fn normalize_file_key(file_path: &str) -> String {
    file_path.replace('\\', "/")
}

fn build_nested_variant_module(
    variant_ident: &syn::Ident,
    wrapper_items: &[proc_macro2::TokenStream],
) -> proc_macro2::TokenStream {
    quote! {
        #[allow(non_snake_case)]
        pub mod #variant_ident {
            #[allow(unused_imports)]
            use super::*;
            #(#wrapper_items)*
        }
    }
}

fn build_root_variant_item(
    outer_enum: &ItemEnum,
    actual_enum_ident: &syn::Ident,
    variant: &syn::Variant,
) -> Result<Option<proc_macro2::TokenStream>, syn::Error> {
    let variant_ident = &variant.ident;
    let (_, ty_generics, _) = outer_enum.generics.split_for_impl();
    let (fn_generics, outer_return_ty, where_clause, can_use_const) =
        wrapper_signature_tokens_with_return_ty(outer_enum, quote! { self::Enum #ty_generics });
    let variant_path = quote! { self::#actual_enum_ident::#variant_ident };

    match &variant.fields {
        Fields::Unit if can_use_const => Ok(Some(quote! {
            #[allow(non_upper_case_globals)]
            pub const #variant_ident: #outer_return_ty = #variant_path;
        })),
        Fields::Unit => Ok(Some(quote! {
            pub fn #variant_ident #fn_generics () -> #outer_return_ty #where_clause {
                #variant_path
            }
        })),
        Fields::Unnamed(fields) => {
            let args: Vec<_> = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let ident = format_ident!("v{i}");
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                })
                .collect();
            let arg_idents: Vec<_> = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let ident = format_ident!("v{i}");
                    quote! { #ident }
                })
                .collect();

            Ok(Some(quote! {
                pub fn #variant_ident #fn_generics (#(#args),*) -> #outer_return_ty #where_clause {
                    #variant_path(#(#arg_idents),*)
                }
            }))
        }
        Fields::Named(_) => Ok(None),
    }
}

fn variant_from_fields(variant: &syn::Variant) -> Vec<&syn::Field> {
    match &variant.fields {
        Fields::Unit => Vec::new(),
        Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .filter(|field| field_has_attr(field, "from"))
            .collect(),
        Fields::Named(fields) => fields
            .named
            .iter()
            .filter(|field| field_has_attr(field, "from"))
            .collect(),
    }
}

fn field_has_attr(field: &syn::Field, attr_name: &str) -> bool {
    field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident(attr_name))
}

fn type_key(ty: &syn::Type) -> String {
    quote!(#ty).to_string()
}

fn type_from_path(path: syn::Path) -> syn::Type {
    syn::Type::Path(syn::TypePath { qself: None, path })
}

fn resolve_enum_info_from_type(
    ty: &syn::Type,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<Option<ResolvedEnumInfo>, syn::Error> {
    let Some(enum_path) = extract_plain_enum_path(ty)? else {
        return Ok(None);
    };
    let module_path_str = resolve_module_path_string(
        &enum_path.module_path,
        enum_path.path_base,
        current_module,
        ty.span(),
    )?;
    let mut resolve_ctx = ResolveCtx {
        current_file,
        module_root,
        current_module,
        enums_by_ident,
        cache,
    };
    let Some((item, marked)) = resolve_enum_from_path(
        &enum_path.module_path,
        enum_path.path_base,
        &enum_path.enum_ident,
        &mut resolve_ctx,
    )?
    else {
        return Ok(None);
    };

    Ok(Some(ResolvedEnumInfo {
        item,
        marked,
        module_path_str: module_path_str.clone(),
        base_path: enum_type_path_from_module(
            &module_path_str,
            &enum_path.enum_ident,
            &enum_path.enum_arguments,
            false,
        ),
        enum_type_path: enum_type_path_from_module(
            &module_path_str,
            &enum_path.enum_ident,
            &enum_path.enum_arguments,
            marked,
        ),
    }))
}

fn canonical_error_source_type(
    ty: &syn::Type,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<syn::Type, syn::Error> {
    let Some(resolved) = resolve_enum_info_from_type(
        ty,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
        cache,
    )?
    else {
        return Ok(ty.clone());
    };
    if resolved.marked {
        Ok(type_from_path(resolved.enum_type_path))
    } else {
        Ok(ty.clone())
    }
}

fn collect_direct_from_type_keys(
    item: &ItemEnum,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<HashSet<String>, syn::Error> {
    let mut keys = HashSet::new();
    for variant in &item.variants {
        for field in variant_from_fields(variant) {
            let ty = canonical_error_source_type(
                &field.ty,
                current_file,
                module_root,
                current_module,
                enums_by_ident,
                cache,
            )?;
            keys.insert(type_key(&ty));
        }
    }
    Ok(keys)
}

fn collect_transitive_from_source_types(
    enum_info: &ResolvedEnumInfo,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
) -> Result<Vec<syn::Type>, syn::Error> {
    let mut collected = Vec::new();
    let mut seen = HashSet::new();
    let mut visited = HashSet::new();
    collect_transitive_from_source_types_inner(
        enum_info,
        current_file,
        module_root,
        cache,
        &mut visited,
        &mut seen,
        &mut collected,
    )?;
    Ok(collected)
}

fn collect_transitive_from_source_types_inner(
    enum_info: &ResolvedEnumInfo,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
    visited: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    collected: &mut Vec<syn::Type>,
) -> Result<(), syn::Error> {
    let enum_key = format!("{}::{}", enum_info.module_path_str, enum_info.item.ident);
    if !visited.insert(enum_key) {
        return Ok(());
    }

    let owner_enums = cache
        .get(&enum_info.module_path_str)
        .cloned()
        .unwrap_or_else(|| {
            let mut map = HashMap::new();
            map.insert(enum_info.item.ident.to_string(), enum_info.item.clone());
            map
        });

    for variant in &enum_info.item.variants {
        for field in variant_from_fields(variant) {
            let resolved = resolve_enum_info_from_type(
                &field.ty,
                current_file,
                module_root,
                &enum_info.module_path_str,
                &owner_enums,
                cache,
            )?;
            let source_ty =
                if let Some(resolved) = resolved.as_ref().filter(|resolved| resolved.marked) {
                    type_from_path(resolved.enum_type_path.clone())
                } else {
                    field.ty.clone()
                };
            let key = type_key(&source_ty);
            if seen.insert(key) {
                collected.push(source_ty);
            }
            if let Some(resolved) = resolved.filter(|resolved| resolved.marked) {
                collect_transitive_from_source_types_inner(
                    &resolved,
                    current_file,
                    module_root,
                    cache,
                    visited,
                    seen,
                    collected,
                )?;
            }
        }
    }

    Ok(())
}

fn build_transitive_from_impls_for_nested_variant(
    outer_item: &ItemEnum,
    variant: &syn::Variant,
    inner_enum: &ResolvedEnumInfo,
    outer_direct_from_type_keys: &HashSet<String>,
    generated_transitive_from_type_keys: &mut HashMap<String, String>,
    resolve_ctx: &mut ResolveCtx<'_>,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    if variant_from_fields(variant).is_empty() {
        return Ok(Vec::new());
    }

    let immediate_inner_ty = canonical_error_source_type(
        &extract_single_tuple_type(variant)?,
        resolve_ctx.current_file,
        resolve_ctx.module_root,
        resolve_ctx.current_module,
        resolve_ctx.enums_by_ident,
        resolve_ctx.cache,
    )?;
    let transitive_source_types = collect_transitive_from_source_types(
        inner_enum,
        resolve_ctx.current_file,
        resolve_ctx.module_root,
        resolve_ctx.cache,
    )?;
    let immediate_inner_type_key = type_key(&immediate_inner_ty);

    let actual_enum_ident = format_ident!("__Nestum{}", outer_item.ident);
    let variant_ident = &variant.ident;
    let (impl_generics, ty_generics, where_clause) = outer_item.generics.split_for_impl();
    let mut impls = Vec::new();

    for source_ty in transitive_source_types {
        let key = type_key(&source_ty);
        if key == immediate_inner_type_key || outer_direct_from_type_keys.contains(&key) {
            continue;
        }
        if let Some(existing_variant) = generated_transitive_from_type_keys.get(&key) {
            if existing_variant != &variant_ident.to_string() {
                return Err(syn::Error::new(
                    variant.span(),
                    format!(
                        "cannot generate transitive From<{}> for {}; \
multiple nested #[from] variants would accept it ({} and {}). \
Add a direct outer conversion or remove one of the nested #[from] paths",
                        quote!(#source_ty),
                        outer_item.ident,
                        existing_variant,
                        variant_ident,
                    ),
                ));
            }
            continue;
        }
        generated_transitive_from_type_keys.insert(key, variant_ident.to_string());

        impls.push(quote! {
            impl #impl_generics ::core::convert::From<#source_ty> for self::#actual_enum_ident #ty_generics #where_clause {
                fn from(value: #source_ty) -> Self {
                    Self::#variant_ident(::core::convert::Into::into(value))
                }
            }
        });
    }

    Ok(impls)
}

fn emit_expanded_enum_module(
    item: &ItemEnum,
    enum_variants: Vec<syn::Variant>,
    root_variants: &[syn::Variant],
    _nested_variant_module_idents: &[syn::Ident],
    nested_variant_modules: Vec<proc_macro2::TokenStream>,
    support_items: Vec<proc_macro2::TokenStream>,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let vis = &item.vis;
    let enum_mod_ident = &item.ident;
    let actual_enum_ident = format_ident!("__Nestum{}", enum_mod_ident);
    let mut rewritten_item = cleaned_enum_item(item, enum_variants);
    rewritten_item.ident = actual_enum_ident.clone();
    let alias_generics = &item.generics;
    let (_, ty_generics, _) = item.generics.split_for_impl();
    let enum_alias_ident = generated_enum_alias_ident();
    let mut root_variant_items = Vec::new();
    let mut root_variant_reexport_idents = Vec::new();
    for variant in root_variants {
        match build_root_variant_item(item, &actual_enum_ident, variant)? {
            Some(item_tokens) => root_variant_items.push(item_tokens),
            None => root_variant_reexport_idents.push(variant.ident.clone()),
        }
    }
    let root_variant_exports = if root_variant_reexport_idents.is_empty() {
        quote! {}
    } else {
        quote! {
            pub use self::#actual_enum_ident::{#(#root_variant_reexport_idents),*};
        }
    };

    Ok(quote! {
        #[allow(non_snake_case)]
        #vis mod #enum_mod_ident {
            #[allow(unused_imports)]
            use super::*;
            #rewritten_item
            // Keep a distinct backing enum so derive macros can target a type, not this module.
            #[allow(type_alias_bounds)]
            #vis type #enum_alias_ident #alias_generics = self::#actual_enum_ident #ty_generics;
            #root_variant_exports
            #(#root_variant_items)*
            #(#nested_variant_modules)*
            #(#support_items)*
        }
    })
}

fn expand_enum_with_context(
    item: ItemEnum,
    enums_by_ident: &EnumMap,
    _marked_enums: &HashSet<String>,
    module_path: &str,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let mut enum_variants = Vec::new();
    let mut root_variants = Vec::new();
    let mut nested_variant_module_idents = Vec::new();
    let mut nested_variant_modules = Vec::new();
    let mut support_items = Vec::new();
    let outer_direct_from_type_keys = collect_direct_from_type_keys(
        &item,
        current_file,
        module_root,
        module_path,
        enums_by_ident,
        cache,
    )?;
    let mut generated_transitive_from_type_keys = HashMap::new();

    for variant in item.variants.iter() {
        let mut variant_clean = variant.clone();
        variant_clean.attrs = strip_nestum_attrs(&variant.attrs);
        let mut is_marked_nested = false;

        let external_path = parse_variant_external_path(&variant.attrs)?;
        if let Some(resolved_inner) = resolve_variant_inner_enum(
            &item,
            variant,
            module_path,
            current_file,
            module_root,
            cache,
        )? {
            if !resolved_inner.marked {
                if let Some(external_path) = external_path {
                    return Err(syn::Error::new(
                        external_path.span(),
                        "external enum must be marked with #[nestum] to enable nesting",
                    ));
                }
                enum_variants.push(variant_clean);
                continue;
            }

            is_marked_nested = true;
            rewrite_variant_type_for_nested(
                &mut variant_clean,
                resolved_inner.enum_type_path.clone(),
            )?;

            let variant_ident = &variant.ident;
            let wrapper_items = build_wrappers(
                &item,
                module_path,
                variant_ident,
                &resolved_inner,
                current_file,
                module_root,
                cache,
            )?;

            nested_variant_module_idents.push(variant_ident.clone());
            nested_variant_modules.push(build_nested_variant_module(variant_ident, &wrapper_items));
            let mut resolve_ctx = ResolveCtx {
                current_file,
                module_root,
                current_module: module_path,
                enums_by_ident,
                cache,
            };
            support_items.extend(build_transitive_from_impls_for_nested_variant(
                &item,
                variant,
                &resolved_inner,
                &outer_direct_from_type_keys,
                &mut generated_transitive_from_type_keys,
                &mut resolve_ctx,
            )?);
        }

        if !is_marked_nested {
            root_variants.push(variant_clean.clone());
        }
        enum_variants.push(variant_clean);
    }

    emit_expanded_enum_module(
        &item,
        enum_variants,
        &root_variants,
        &nested_variant_module_idents,
        nested_variant_modules,
        support_items,
    )
}

fn build_wrappers(
    outer_enum: &ItemEnum,
    outer_module_path: &str,
    outer_variant: &syn::Ident,
    inner_enum: &ResolvedEnumInfo,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    let outer_base_path = enum_type_path_from_module(
        outer_module_path,
        &outer_enum.ident,
        &syn::PathArguments::None,
        false,
    );
    let outer_return_ty = outer_return_ty_tokens(outer_enum, &outer_base_path);
    let outer_variant_path =
        enum_variant_path_from_base(&outer_base_path, &outer_enum.ident, true, outer_variant);

    build_recursive_wrapper_items(
        outer_enum,
        &outer_return_ty,
        &[outer_variant_path],
        inner_enum,
        current_file,
        module_root,
        cache,
    )
}

fn build_recursive_wrapper_items(
    outer_enum: &ItemEnum,
    outer_return_ty: &proc_macro2::TokenStream,
    constructor_chain: &[syn::Path],
    current_enum: &ResolvedEnumInfo,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    let mut items = Vec::new();

    for inner_variant in current_enum.item.variants.iter() {
        let inner_variant_path = enum_variant_path_from_base(
            &current_enum.base_path,
            &current_enum.item.ident,
            current_enum.marked,
            &inner_variant.ident,
        );
        let nested_enum = resolve_variant_inner_enum(
            &current_enum.item,
            inner_variant,
            &current_enum.module_path_str,
            current_file,
            module_root,
            cache,
        )?;

        let mut wrapper_variant = inner_variant.clone();
        if let Some(nested_enum) = nested_enum
            .as_ref()
            .filter(|nested_enum| nested_enum.marked)
        {
            rewrite_variant_type_for_nested(
                &mut wrapper_variant,
                nested_enum.enum_type_path.clone(),
            )?;
        }

        items.push(build_wrapper_from_syn_variant_with_chain(
            outer_enum,
            outer_return_ty,
            constructor_chain,
            &wrapper_variant,
            &inner_variant_path,
        ));

        let Some(nested_enum) = nested_enum.filter(|nested_enum| nested_enum.marked) else {
            continue;
        };

        let mut nested_constructor_chain = constructor_chain.to_vec();
        nested_constructor_chain.push(inner_variant_path);
        let nested_items = build_recursive_wrapper_items(
            outer_enum,
            outer_return_ty,
            &nested_constructor_chain,
            &nested_enum,
            current_file,
            module_root,
            cache,
        )?;
        items.push(build_nested_variant_module(
            &inner_variant.ident,
            &nested_items,
        ));
    }

    Ok(items)
}

fn apply_constructor_chain(
    constructor_chain: &[syn::Path],
    leaf_expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    constructor_chain
        .iter()
        .rev()
        .fold(leaf_expr, |expr, constructor_path| {
            quote! { #constructor_path(#expr) }
        })
}

fn build_wrapper_from_syn_variant_with_chain(
    outer_enum: &ItemEnum,
    outer_return_ty: &proc_macro2::TokenStream,
    constructor_chain: &[syn::Path],
    inner_variant: &syn::Variant,
    inner_variant_path: &syn::Path,
) -> proc_macro2::TokenStream {
    let inner_ident = &inner_variant.ident;
    let (fn_generics, outer_return_ty, where_clause, can_use_const) =
        wrapper_signature_tokens_with_return_ty(outer_enum, outer_return_ty.clone());

    match &inner_variant.fields {
        Fields::Unit if can_use_const => {
            let expr = apply_constructor_chain(constructor_chain, quote! { #inner_variant_path });
            quote! {
                #[allow(non_upper_case_globals)]
                pub const #inner_ident: #outer_return_ty = #expr;
            }
        }
        Fields::Unit => {
            let expr = apply_constructor_chain(constructor_chain, quote! { #inner_variant_path });
            quote! {
                pub fn #inner_ident #fn_generics () -> #outer_return_ty #where_clause {
                    #expr
                }
            }
        }
        Fields::Unnamed(fields) => {
            let args: Vec<_> = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let ident = format_ident!("v{i}");
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                })
                .collect();
            let arg_idents: Vec<_> = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let ident = format_ident!("v{i}");
                    quote! { #ident }
                })
                .collect();
            let expr = apply_constructor_chain(
                constructor_chain,
                quote! { #inner_variant_path(#(#arg_idents),*) },
            );
            quote! {
                pub fn #inner_ident #fn_generics (#(#args),*) -> #outer_return_ty #where_clause {
                    #expr
                }
            }
        }
        Fields::Named(fields) => {
            let args: Vec<_> = fields
                .named
                .iter()
                .map(|f| {
                    let ident = f
                        .ident
                        .as_ref()
                        .expect("named field should have an identifier");
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                })
                .collect();
            let arg_idents: Vec<_> = fields
                .named
                .iter()
                .map(|f| {
                    let ident = f
                        .ident
                        .as_ref()
                        .expect("named field should have an identifier");
                    quote! { #ident }
                })
                .collect();
            let expr = apply_constructor_chain(
                constructor_chain,
                quote! { #inner_variant_path { #(#arg_idents),* } },
            );
            quote! {
                pub fn #inner_ident #fn_generics (#(#args),*) -> #outer_return_ty #where_clause {
                    #expr
                }
            }
        }
    }
}

fn wrapper_signature_tokens_with_return_ty(
    outer_enum: &ItemEnum,
    outer_return_ty: proc_macro2::TokenStream,
) -> (
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    bool,
) {
    let mut fn_generics = outer_enum.generics.clone();
    for param in &mut fn_generics.params {
        match param {
            syn::GenericParam::Lifetime(_) => {}
            syn::GenericParam::Type(param) => param.default = None,
            syn::GenericParam::Const(param) => param.default = None,
        }
    }

    let fn_generics = if fn_generics.params.is_empty() {
        quote! {}
    } else {
        let params = &fn_generics.params;
        quote! { <#params> }
    };
    let (_, _, where_clause) = outer_enum.generics.split_for_impl();
    (
        fn_generics,
        outer_return_ty,
        quote! { #where_clause },
        outer_enum.generics.params.is_empty(),
    )
}

fn rewrite_variant_type_for_nested(
    variant: &mut syn::Variant,
    type_path: syn::Path,
) -> Result<(), syn::Error> {
    let syn::Fields::Unnamed(fields) = &mut variant.fields else {
        return Ok(());
    };
    if fields.unnamed.len() != 1 {
        return Ok(());
    }

    let ty = syn::Type::Path(syn::TypePath {
        qself: None,
        path: type_path,
    });
    fields.unnamed[0].ty = ty;
    Ok(())
}

fn path_segment(ident: &syn::Ident, arguments: syn::PathArguments) -> syn::PathSegment {
    syn::PathSegment {
        ident: ident.clone(),
        arguments,
    }
}

fn generated_enum_alias_ident() -> syn::Ident {
    format_ident!("Enum")
}

fn append_enum_type_segments(
    segments: &mut Vec<syn::PathSegment>,
    enum_ident: &syn::Ident,
    enum_arguments: &syn::PathArguments,
    marked: bool,
) {
    if marked {
        segments.push(path_segment(enum_ident, syn::PathArguments::None));
        segments.push(path_segment(
            &generated_enum_alias_ident(),
            enum_arguments.clone(),
        ));
    } else {
        segments.push(path_segment(enum_ident, enum_arguments.clone()));
    }
}

fn enum_type_path_from_module(
    module_path: &str,
    enum_ident: &syn::Ident,
    enum_arguments: &syn::PathArguments,
    marked: bool,
) -> syn::Path {
    let mut segments = absolute_module_idents(module_path)
        .into_iter()
        .map(|ident| path_segment(&ident, syn::PathArguments::None))
        .collect::<Vec<_>>();
    append_enum_type_segments(&mut segments, enum_ident, enum_arguments, marked);
    syn::Path {
        leading_colon: None,
        segments: Punctuated::from_iter(segments),
    }
}

fn enum_variant_path_from_base(
    base_path: &syn::Path,
    _enum_ident: &syn::Ident,
    marked: bool,
    variant_ident: &syn::Ident,
) -> syn::Path {
    let mut segments = base_path.segments.iter().cloned().collect::<Vec<_>>();
    if marked {
        segments.push(path_segment(
            &generated_enum_alias_ident(),
            syn::PathArguments::None,
        ));
    }
    segments.push(path_segment(variant_ident, syn::PathArguments::None));
    syn::Path {
        leading_colon: base_path.leading_colon,
        segments: Punctuated::from_iter(segments),
    }
}

fn outer_return_ty_tokens(
    outer_enum: &ItemEnum,
    outer_base_path: &syn::Path,
) -> proc_macro2::TokenStream {
    let (_, ty_generics, _) = outer_enum.generics.split_for_impl();
    quote! { #outer_base_path::Enum #ty_generics }
}

fn plain_enum_path_from_path(path: &syn::Path) -> Result<PlainEnumPath, syn::Error> {
    let ty = syn::Type::Path(syn::TypePath {
        qself: None,
        path: path.clone(),
    });
    extract_plain_enum_path(&ty)?.ok_or_else(|| {
        syn::Error::new(
            path.span(),
            "external path must include an enum ident, e.g. crate::foo::Enum",
        )
    })
}

fn resolve_variant_inner_enum(
    owner_enum: &ItemEnum,
    variant: &syn::Variant,
    owner_module_path: &str,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
) -> Result<Option<ResolvedEnumInfo>, syn::Error> {
    if !cache.contains_key(owner_module_path) {
        load_module_enums(
            variant.span(),
            owner_module_path,
            current_file,
            module_root,
            cache,
        )?;
    }
    let owner_enums = cache.get(owner_module_path).cloned().ok_or_else(|| {
        syn::Error::new(
            owner_enum.span(),
            format!("no enums found for module path {owner_module_path}"),
        )
    })?;
    let mut resolve_ctx = ResolveCtx {
        current_file,
        module_root,
        current_module: owner_module_path,
        enums_by_ident: &owner_enums,
        cache,
    };

    if let Some(external_path) = parse_variant_external_path(&variant.attrs)? {
        let inner_ty = extract_single_tuple_type(variant).map_err(|_| {
            syn::Error::new(
                variant.span(),
                format!(
                    "variant {}::{} uses #[nestum(external = \"...\")], \
but is not a single-field tuple variant",
                    owner_enum.ident, variant.ident
                ),
            )
        })?;

        let inner_ident = extract_simple_ident(&inner_ty).map_err(|_| {
            syn::Error::new(
                inner_ty.span(),
                "nested enum type must be a simple ident when using #[nestum(external = \"...\")]; \
use a bare enum name in the field",
            )
        })?;

        let external_parts = plain_enum_path_from_path(&external_path)?;
        if inner_ident != external_parts.enum_ident {
            return Err(syn::Error::new(
                inner_ty.span(),
                format!(
                    "field type {} does not match external enum path {}; \
use the enum ident as the field type",
                    inner_ident,
                    external_path_to_string(&external_path),
                ),
            ));
        }

        let module_path_str = resolve_module_path_string(
            &external_parts.module_path,
            external_parts.path_base,
            owner_module_path,
            external_path.span(),
        );
        let module_path_str = module_path_str?;
        let Some((item, marked)) = resolve_enum_from_path(
            &external_parts.module_path,
            external_parts.path_base,
            &external_parts.enum_ident,
            &mut resolve_ctx,
        )?
        else {
            return Err(syn::Error::new(
                external_path.span(),
                format!(
                    "external enum {} not found; \
ensure the module path exists and the enum is declared in that module",
                    external_path_to_string(&external_path),
                ),
            ));
        };

        return Ok(Some(ResolvedEnumInfo {
            item,
            marked,
            module_path_str: module_path_str.clone(),
            base_path: enum_type_path_from_module(
                &module_path_str,
                &external_parts.enum_ident,
                &syn::PathArguments::None,
                false,
            ),
            enum_type_path: enum_type_path_from_module(
                &module_path_str,
                &external_parts.enum_ident,
                &syn::PathArguments::None,
                marked,
            ),
        }));
    }

    let inner_ty = match extract_single_tuple_type(variant) {
        Ok(inner_ty) => inner_ty,
        Err(_) => return Ok(None),
    };
    let Some(inner_path) = extract_plain_enum_path(&inner_ty)? else {
        return Ok(None);
    };

    let module_path_str = resolve_module_path_string(
        &inner_path.module_path,
        inner_path.path_base,
        owner_module_path,
        inner_ty.span(),
    );
    let module_path_str = module_path_str?;
    let Some((item, marked)) = resolve_enum_from_path(
        &inner_path.module_path,
        inner_path.path_base,
        &inner_path.enum_ident,
        &mut resolve_ctx,
    )?
    else {
        return Ok(None);
    };

    Ok(Some(ResolvedEnumInfo {
        item,
        marked,
        module_path_str: module_path_str.clone(),
        base_path: enum_type_path_from_module(
            &module_path_str,
            &inner_path.enum_ident,
            &syn::PathArguments::None,
            false,
        ),
        enum_type_path: enum_type_path_from_module(
            &module_path_str,
            &inner_path.enum_ident,
            &inner_path.enum_arguments,
            marked,
        ),
    }))
}

fn rewrite_pat(
    pat: Pat,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<Pat, syn::Error> {
    match pat {
        Pat::Ident(mut pat_ident) => {
            if let Some((at_token, subpat)) = pat_ident.subpat.take() {
                let subpat = rewrite_pat(
                    *subpat,
                    current_file,
                    module_root,
                    current_module,
                    enums_by_ident,
                    cache,
                )?;
                pat_ident.subpat = Some((at_token, Box::new(subpat)));
            }
            Ok(Pat::Ident(pat_ident))
        }
        Pat::Paren(mut pat_paren) => {
            pat_paren.pat = Box::new(rewrite_pat(
                *pat_paren.pat,
                current_file,
                module_root,
                current_module,
                enums_by_ident,
                cache,
            )?);
            Ok(Pat::Paren(pat_paren))
        }
        Pat::Path(pat_path) => rewrite_pat_path(
            pat_path,
            current_file,
            module_root,
            current_module,
            enums_by_ident,
            cache,
        ),
        Pat::TupleStruct(pat_tuple) => rewrite_pat_tuple_struct(
            pat_tuple,
            current_file,
            module_root,
            current_module,
            enums_by_ident,
            cache,
        ),
        Pat::Reference(mut pat_reference) => {
            pat_reference.pat = Box::new(rewrite_pat(
                *pat_reference.pat,
                current_file,
                module_root,
                current_module,
                enums_by_ident,
                cache,
            )?);
            Ok(Pat::Reference(pat_reference))
        }
        Pat::Slice(mut pat_slice) => {
            let mut new_elems = Vec::with_capacity(pat_slice.elems.len());
            for pat in pat_slice.elems {
                new_elems.push(rewrite_pat(
                    pat,
                    current_file,
                    module_root,
                    current_module,
                    enums_by_ident,
                    cache,
                )?);
            }
            pat_slice.elems = Punctuated::from_iter(new_elems);
            Ok(Pat::Slice(pat_slice))
        }
        Pat::Tuple(mut pat_tuple) => {
            let mut new_elems = Vec::with_capacity(pat_tuple.elems.len());
            for pat in pat_tuple.elems {
                new_elems.push(rewrite_pat(
                    pat,
                    current_file,
                    module_root,
                    current_module,
                    enums_by_ident,
                    cache,
                )?);
            }
            pat_tuple.elems = Punctuated::from_iter(new_elems);
            Ok(Pat::Tuple(pat_tuple))
        }
        Pat::Struct(pat_struct) => rewrite_pat_struct(
            pat_struct,
            current_file,
            module_root,
            current_module,
            enums_by_ident,
            cache,
        ),
        Pat::Type(mut pat_type) => {
            pat_type.pat = Box::new(rewrite_pat(
                *pat_type.pat,
                current_file,
                module_root,
                current_module,
                enums_by_ident,
                cache,
            )?);
            Ok(Pat::Type(pat_type))
        }
        Pat::Or(PatOr {
            attrs,
            leading_vert,
            cases,
        }) => {
            let mut new_pats = Vec::with_capacity(cases.len());
            for pat in cases {
                new_pats.push(rewrite_pat(
                    pat,
                    current_file,
                    module_root,
                    current_module,
                    enums_by_ident,
                    cache,
                )?);
            }
            Ok(Pat::Or(PatOr {
                attrs,
                leading_vert,
                cases: Punctuated::from_iter(new_pats),
            }))
        }
        other => Ok(other),
    }
}

fn rewrite_pat_path(
    pat_path: PatPath,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<Pat, syn::Error> {
    if pat_path.qself.is_some() {
        return Ok(Pat::Path(pat_path));
    }

    let Some(resolved_path) = resolve_nested_match_path(
        &pat_path.path,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
        cache,
    )?
    else {
        return Ok(Pat::Path(pat_path));
    };

    let leaf_step = resolved_path
        .steps
        .last()
        .expect("resolved nested path should include at least one variant");
    let leaf_path = enum_variant_path_from_base(
        &leaf_step.owner.base_path,
        &leaf_step.owner.item.ident,
        leaf_step.owner.marked,
        &leaf_step.variant.ident,
    );
    let leaf_pat = Pat::Path(PatPath {
        attrs: pat_path.attrs,
        qself: pat_path.qself,
        path: leaf_path,
    });
    Ok(build_wrapped_nested_pat(&resolved_path, leaf_pat))
}

fn rewrite_pat_tuple_struct(
    pat_tuple: PatTupleStruct,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<Pat, syn::Error> {
    if pat_tuple.qself.is_some() {
        let mut pat_tuple = pat_tuple;
        let mut new_elems = Vec::with_capacity(pat_tuple.elems.len());
        for pat in pat_tuple.elems {
            new_elems.push(rewrite_pat(
                pat,
                current_file,
                module_root,
                current_module,
                enums_by_ident,
                cache,
            )?);
        }
        pat_tuple.elems = Punctuated::from_iter(new_elems);
        return Ok(Pat::TupleStruct(pat_tuple));
    }

    let mut elems = Vec::with_capacity(pat_tuple.elems.len());
    for pat in pat_tuple.elems {
        elems.push(rewrite_pat(
            pat,
            current_file,
            module_root,
            current_module,
            enums_by_ident,
            cache,
        )?);
    }

    let Some(resolved_path) = resolve_nested_match_path(
        &pat_tuple.path,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
        cache,
    )?
    else {
        return Ok(Pat::TupleStruct(PatTupleStruct {
            elems: Punctuated::from_iter(elems),
            ..pat_tuple
        }));
    };

    let leaf_step = resolved_path
        .steps
        .last()
        .expect("resolved nested path should include at least one variant");
    let leaf_path = enum_variant_path_from_base(
        &leaf_step.owner.base_path,
        &leaf_step.owner.item.ident,
        leaf_step.owner.marked,
        &leaf_step.variant.ident,
    );
    let leaf_pat = Pat::TupleStruct(PatTupleStruct {
        attrs: pat_tuple.attrs,
        qself: pat_tuple.qself,
        path: leaf_path,
        paren_token: pat_tuple.paren_token,
        elems: Punctuated::from_iter(elems),
    });
    Ok(build_wrapped_nested_pat(&resolved_path, leaf_pat))
}

fn rewrite_pat_struct(
    pat_struct: PatStruct,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<Pat, syn::Error> {
    if pat_struct.qself.is_some() {
        let mut pat_struct = pat_struct;
        for field in &mut pat_struct.fields {
            *field.pat = rewrite_pat(
                (*field.pat).clone(),
                current_file,
                module_root,
                current_module,
                enums_by_ident,
                cache,
            )?;
        }
        return Ok(Pat::Struct(pat_struct));
    }

    let mut fields = pat_struct.fields.clone();
    for field in &mut fields {
        *field.pat = rewrite_pat(
            (*field.pat).clone(),
            current_file,
            module_root,
            current_module,
            enums_by_ident,
            cache,
        )?;
    }

    let Some(resolved_path) = resolve_nested_match_path(
        &pat_struct.path,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
        cache,
    )?
    else {
        return Ok(Pat::Struct(PatStruct {
            fields,
            ..pat_struct
        }));
    };

    let leaf_step = resolved_path
        .steps
        .last()
        .expect("resolved nested path should include at least one variant");
    let leaf_path = enum_variant_path_from_base(
        &leaf_step.owner.base_path,
        &leaf_step.owner.item.ident,
        leaf_step.owner.marked,
        &leaf_step.variant.ident,
    );
    let leaf_pat = Pat::Struct(PatStruct {
        attrs: pat_struct.attrs,
        qself: pat_struct.qself,
        path: leaf_path,
        brace_token: pat_struct.brace_token,
        fields,
        rest: pat_struct.rest,
    });
    Ok(build_wrapped_nested_pat(&resolved_path, leaf_pat))
}

fn resolve_enum_info_from_path(
    module_path: &[syn::Ident],
    path_base: ModulePathBase,
    enum_ident: &syn::Ident,
    ctx: &PathResolveCtx<'_>,
    cache: &mut ModuleEnumCache,
) -> Result<Option<ResolvedEnumInfo>, syn::Error> {
    let module_path_str = resolve_module_path_string(
        module_path,
        path_base,
        ctx.current_module,
        enum_ident.span(),
    )?;
    let mut resolve_ctx = ResolveCtx {
        current_file: ctx.current_file,
        module_root: ctx.module_root,
        current_module: ctx.current_module,
        enums_by_ident: ctx.enums_by_ident,
        cache,
    };
    let Some((item, marked)) =
        resolve_enum_from_path(module_path, path_base, enum_ident, &mut resolve_ctx)?
    else {
        return Ok(None);
    };

    Ok(Some(ResolvedEnumInfo {
        item,
        marked,
        module_path_str: module_path_str.clone(),
        base_path: enum_type_path_from_module(
            &module_path_str,
            enum_ident,
            &syn::PathArguments::None,
            false,
        ),
        enum_type_path: enum_type_path_from_module(
            &module_path_str,
            enum_ident,
            &syn::PathArguments::None,
            marked,
        ),
    }))
}

fn resolve_nested_match_path(
    path: &syn::Path,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &EnumMap,
    cache: &mut ModuleEnumCache,
) -> Result<Option<ResolvedMatchPath>, syn::Error> {
    if path
        .segments
        .iter()
        .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
    {
        return Ok(None);
    }

    let segments: Vec<_> = path.segments.iter().map(|s| s.ident.clone()).collect();
    if segments.len() < 2 {
        return Ok(None);
    }
    let path_ctx = PathResolveCtx {
        current_file,
        module_root,
        current_module,
        enums_by_ident,
    };

    for outer_idx in (0..segments.len() - 1).rev() {
        let raw_module_path = segments[..outer_idx].to_vec();
        let (path_base, module_path) = split_module_path_base(&raw_module_path);
        let outer_enum = &segments[outer_idx];
        let variant_chain = &segments[outer_idx + 1..];

        let Some(mut current_enum) =
            resolve_enum_info_from_path(&module_path, path_base, outer_enum, &path_ctx, cache)
                .unwrap_or_default()
        else {
            continue;
        };
        if !current_enum.marked {
            continue;
        }

        if variant_chain.first() == Some(&current_enum.item.ident)
            && !current_enum
                .item
                .variants
                .iter()
                .any(|variant| variant.ident == current_enum.item.ident)
        {
            continue;
        }

        let mut steps = Vec::new();
        for (idx, variant_ident) in variant_chain.iter().enumerate() {
            let Some(variant) = current_enum
                .item
                .variants
                .iter()
                .find(|variant| variant.ident == *variant_ident)
                .cloned()
            else {
                return Err(syn::Error::new(
                    path.span(),
                    format!(
                        "variant {} not found on enum {}",
                        variant_ident, current_enum.item.ident
                    ),
                ));
            };

            let is_last = idx + 1 == variant_chain.len();
            steps.push(ResolvedVariantStep {
                owner: current_enum.clone(),
                variant: variant.clone(),
            });
            if is_last {
                return Ok(Some(ResolvedMatchPath { steps }));
            }

            let Some(next_enum) = resolve_variant_inner_enum(
                &current_enum.item,
                &variant,
                &current_enum.module_path_str,
                current_file,
                module_root,
                cache,
            )?
            else {
                return Err(unresolved_nested_branch_error(
                    &current_enum.item,
                    &variant,
                    path.span(),
                ));
            };
            if !next_enum.marked {
                return Err(syn::Error::new(
                    path.span(),
                    format!(
                        "inner enum {} is not marked with #[nestum]; \
only #[nestum] enums support nested match patterns",
                        next_enum.item.ident
                    ),
                ));
            }

            current_enum = next_enum;
        }
    }

    Ok(None)
}

fn unresolved_nested_branch_error(
    owner_enum: &ItemEnum,
    variant: &syn::Variant,
    span: proc_macro2::Span,
) -> syn::Error {
    let default = || {
        syn::Error::new(
            span,
            format!(
                "variant {} on enum {} does not wrap a #[nestum] enum",
                variant.ident, owner_enum.ident
            ),
        )
    };

    let Ok(inner_ty) = extract_single_tuple_type(variant) else {
        return default();
    };
    let Ok(Some(_)) = extract_plain_enum_path(&inner_ty) else {
        return default();
    };

    syn::Error::new(
        span,
        format!(
            "variant {} on enum {} points at inner type {}, but nestum could not resolve it as a crate-local #[nestum] enum from parsed source; nested paths require a supported crate-local #[nestum] enum. macro-generated local enums, external crates, and unsupported module layouts cannot be used as nested branches",
            variant.ident,
            owner_enum.ident,
            quote!(#inner_ty)
        ),
    )
}

fn build_wrapped_nested_pat(resolved_path: &ResolvedMatchPath, leaf_pat: Pat) -> Pat {
    resolved_path
        .steps
        .iter()
        .rev()
        .skip(1)
        .fold(leaf_pat, |nested_pat, step| {
            let outer_path = enum_variant_path_from_base(
                &step.owner.base_path,
                &step.owner.item.ident,
                step.owner.marked,
                &step.variant.ident,
            );

            Pat::TupleStruct(PatTupleStruct {
                attrs: Vec::new(),
                qself: None,
                path: outer_path,
                paren_token: Default::default(),
                elems: Punctuated::from_iter(std::iter::once(nested_pat)),
            })
        })
}

struct NestedExprRewriter<'a> {
    current_file: &'a str,
    module_root: &'a std::path::Path,
    current_module: &'a str,
    enums_by_ident: &'a EnumMap,
    cache: &'a mut ModuleEnumCache,
    error: Option<syn::Error>,
}

impl NestedExprRewriter<'_> {
    fn set_error(&mut self, err: syn::Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn rewrite_expr_path(&mut self, expr_path: ExprPath) -> Result<Option<Expr>, syn::Error> {
        if expr_path.qself.is_some() {
            return Ok(None);
        }

        let Some(resolved_path) = resolve_nested_match_path(
            &expr_path.path,
            self.current_file,
            self.module_root,
            self.current_module,
            self.enums_by_ident,
            self.cache,
        )?
        else {
            return Ok(None);
        };

        if !matches!(
            resolved_path
                .steps
                .last()
                .expect("resolved nested path should include at least one variant")
                .variant
                .fields,
            Fields::Unit
        ) {
            return Ok(None);
        }

        let leaf_path = leaf_variant_path(&resolved_path);
        let leaf_expr = Expr::Path(ExprPath {
            attrs: expr_path.attrs,
            qself: expr_path.qself,
            path: leaf_path,
        });
        Ok(Some(build_wrapped_nested_expr(&resolved_path, leaf_expr)))
    }

    fn rewrite_expr_call(&mut self, expr_call: ExprCall) -> Result<Option<Expr>, syn::Error> {
        let Expr::Path(func_path) = expr_call.func.as_ref() else {
            return Ok(None);
        };
        if func_path.qself.is_some() {
            return Ok(None);
        }

        let Some(resolved_path) = resolve_nested_match_path(
            &func_path.path,
            self.current_file,
            self.module_root,
            self.current_module,
            self.enums_by_ident,
            self.cache,
        )?
        else {
            return Ok(None);
        };

        if !matches!(
            resolved_path
                .steps
                .last()
                .expect("resolved nested path should include at least one variant")
                .variant
                .fields,
            Fields::Unnamed(_)
        ) {
            return Ok(None);
        }

        let leaf_path = leaf_variant_path(&resolved_path);
        let leaf_expr = Expr::Call(ExprCall {
            attrs: expr_call.attrs,
            func: Box::new(Expr::Path(ExprPath {
                attrs: Vec::new(),
                qself: None,
                path: leaf_path,
            })),
            paren_token: expr_call.paren_token,
            args: expr_call.args,
        });
        Ok(Some(build_wrapped_nested_expr(&resolved_path, leaf_expr)))
    }

    fn rewrite_expr_struct(&mut self, expr_struct: ExprStruct) -> Result<Option<Expr>, syn::Error> {
        if expr_struct.qself.is_some() {
            return Ok(None);
        }

        let Some(resolved_path) = resolve_nested_match_path(
            &expr_struct.path,
            self.current_file,
            self.module_root,
            self.current_module,
            self.enums_by_ident,
            self.cache,
        )?
        else {
            return Ok(None);
        };

        if !matches!(
            resolved_path
                .steps
                .last()
                .expect("resolved nested path should include at least one variant")
                .variant
                .fields,
            Fields::Named(_)
        ) {
            return Ok(None);
        }

        let leaf_path = leaf_variant_path(&resolved_path);
        let leaf_expr = Expr::Struct(ExprStruct {
            attrs: expr_struct.attrs,
            qself: expr_struct.qself,
            path: leaf_path,
            brace_token: expr_struct.brace_token,
            fields: expr_struct.fields,
            dot2_token: expr_struct.dot2_token,
            rest: expr_struct.rest,
        });
        Ok(Some(build_wrapped_nested_expr(&resolved_path, leaf_expr)))
    }
}

impl VisitMut for NestedExprRewriter<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if self.error.is_some() {
            return;
        }

        match expr {
            Expr::Match(expr_match) => {
                visit_mut::visit_expr_match_mut(self, expr_match);
                if self.error.is_some() {
                    return;
                }
                for arm in &mut expr_match.arms {
                    match rewrite_pat(
                        arm.pat.clone(),
                        self.current_file,
                        self.module_root,
                        self.current_module,
                        self.enums_by_ident,
                        self.cache,
                    ) {
                        Ok(pat) => arm.pat = pat,
                        Err(err) => {
                            self.set_error(err);
                            return;
                        }
                    }
                }
            }
            Expr::Path(expr_path) => {
                visit_mut::visit_expr_path_mut(self, expr_path);
                if self.error.is_some() {
                    return;
                }
                match self.rewrite_expr_path(expr_path.clone()) {
                    Ok(Some(new_expr)) => *expr = new_expr,
                    Ok(None) => {}
                    Err(err) => self.set_error(err),
                }
            }
            Expr::Call(expr_call) => {
                visit_mut::visit_expr_call_mut(self, expr_call);
                if self.error.is_some() {
                    return;
                }
                match self.rewrite_expr_call(expr_call.clone()) {
                    Ok(Some(new_expr)) => *expr = new_expr,
                    Ok(None) => {}
                    Err(err) => self.set_error(err),
                }
            }
            Expr::Struct(expr_struct) => {
                visit_mut::visit_expr_struct_mut(self, expr_struct);
                if self.error.is_some() {
                    return;
                }
                match self.rewrite_expr_struct(expr_struct.clone()) {
                    Ok(Some(new_expr)) => *expr = new_expr,
                    Ok(None) => {}
                    Err(err) => self.set_error(err),
                }
            }
            Expr::Macro(expr_macro) => self.visit_expr_macro_mut(expr_macro),
            _ => visit_mut::visit_expr_mut(self, expr),
        }
    }

    fn visit_expr_let_mut(&mut self, expr_let: &mut ExprLet) {
        if self.error.is_some() {
            return;
        }

        visit_mut::visit_expr_let_mut(self, expr_let);
        if self.error.is_some() {
            return;
        }

        match rewrite_pat(
            (*expr_let.pat).clone(),
            self.current_file,
            self.module_root,
            self.current_module,
            self.enums_by_ident,
            self.cache,
        ) {
            Ok(pat) => *expr_let.pat = pat,
            Err(err) => self.set_error(err),
        }
    }

    fn visit_local_mut(&mut self, local: &mut Local) {
        if self.error.is_some() {
            return;
        }

        visit_mut::visit_local_mut(self, local);
        if self.error.is_some() {
            return;
        }

        match rewrite_pat(
            local.pat.clone(),
            self.current_file,
            self.module_root,
            self.current_module,
            self.enums_by_ident,
            self.cache,
        ) {
            Ok(pat) => local.pat = pat,
            Err(err) => self.set_error(err),
        }
    }

    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if self.error.is_some() {
            return;
        }

        match stmt {
            Stmt::Macro(stmt_macro) => {
                let mut expr_macro = ExprMacro {
                    attrs: stmt_macro.attrs.clone(),
                    mac: stmt_macro.mac.clone(),
                };
                self.visit_expr_macro_mut(&mut expr_macro);
                if self.error.is_some() {
                    return;
                }
                stmt_macro.mac = expr_macro.mac;
            }
            _ => visit_mut::visit_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_macro_mut(&mut self, expr_macro: &mut ExprMacro) {
        if self.error.is_some() {
            return;
        }

        if is_matches_macro_path(&expr_macro.mac.path) {
            let mut input = match syn::parse2::<MatchesMacroInput>(expr_macro.mac.tokens.clone()) {
                Ok(input) => input,
                Err(err) => {
                    self.set_error(syn::Error::new(
                        expr_macro.mac.span(),
                        format!(
                            "unsupported matches! syntax inside nested! or #[nestum_scope]: {err}; \
use matches!(expr, PATTERN) or matches!(expr, PATTERN if guard)"
                        ),
                    ));
                    return;
                }
            };

            self.visit_expr_mut(&mut input.expr);
            if self.error.is_some() {
                return;
            }

            match rewrite_pat(
                input.pat.clone(),
                self.current_file,
                self.module_root,
                self.current_module,
                self.enums_by_ident,
                self.cache,
            ) {
                Ok(pat) => input.pat = pat,
                Err(err) => {
                    self.set_error(err);
                    return;
                }
            }

            if let Some((_, guard)) = &mut input.guard {
                self.visit_expr_mut(guard);
                if self.error.is_some() {
                    return;
                }
            }

            let expr = &input.expr;
            let pat = &input.pat;
            let guard = input
                .guard
                .as_ref()
                .map(|(if_token, guard)| quote!(#if_token #guard));
            expr_macro.mac.tokens = quote!(#expr, #pat #guard);
            return;
        }

        if is_assert_macro_path(&expr_macro.mac.path) {
            let mut input = match syn::parse2::<AssertMacroInput>(expr_macro.mac.tokens.clone()) {
                Ok(input) => input,
                Err(err) => {
                    self.set_error(syn::Error::new(
                        expr_macro.mac.span(),
                        format!(
                            "unsupported assert! syntax inside nested! or #[nestum_scope]: {err}; \
use assert!(expr) or assert!(expr, ...)"
                        ),
                    ));
                    return;
                }
            };

            self.visit_expr_mut(&mut input.expr);
            if self.error.is_some() {
                return;
            }

            let expr = &input.expr;
            let trailing = input
                .trailing
                .as_ref()
                .map(|(comma, rest)| quote!(#comma #rest));
            expr_macro.mac.tokens = quote!(#expr #trailing);
            return;
        }

        if is_binary_assert_macro_path(&expr_macro.mac.path) {
            let mut input = match syn::parse2::<BinaryAssertMacroInput>(
                expr_macro.mac.tokens.clone(),
            ) {
                Ok(input) => input,
                Err(err) => {
                    self.set_error(syn::Error::new(
                            expr_macro.mac.span(),
                            format!(
                                "unsupported assert_eq!/assert_ne! syntax inside nested! or #[nestum_scope]: {err}; \
use assert_eq!(left, right) or assert_eq!(left, right, ...)"
                            ),
                        ));
                    return;
                }
            };

            self.visit_expr_mut(&mut input.left);
            if self.error.is_some() {
                return;
            }

            self.visit_expr_mut(&mut input.right);
            if self.error.is_some() {
                return;
            }

            let left = &input.left;
            let right = &input.right;
            let trailing = input
                .trailing
                .as_ref()
                .map(|(comma, rest)| quote!(#comma #rest));
            expr_macro.mac.tokens = quote!(#left, #right #trailing);
            return;
        }

        visit_mut::visit_expr_macro_mut(self, expr_macro);
    }
}

fn leaf_variant_path(resolved_path: &ResolvedMatchPath) -> syn::Path {
    let leaf_step = resolved_path
        .steps
        .last()
        .expect("resolved nested path should include at least one variant");
    enum_variant_path_from_base(
        &leaf_step.owner.base_path,
        &leaf_step.owner.item.ident,
        leaf_step.owner.marked,
        &leaf_step.variant.ident,
    )
}

fn build_wrapped_nested_expr(resolved_path: &ResolvedMatchPath, leaf_expr: Expr) -> Expr {
    resolved_path
        .steps
        .iter()
        .rev()
        .skip(1)
        .fold(leaf_expr, |nested_expr, step| {
            let outer_path = enum_variant_path_from_base(
                &step.owner.base_path,
                &step.owner.item.ident,
                step.owner.marked,
                &step.variant.ident,
            );
            Expr::Call(ExprCall {
                attrs: Vec::new(),
                func: Box::new(Expr::Path(ExprPath {
                    attrs: Vec::new(),
                    qself: None,
                    path: outer_path,
                })),
                paren_token: Default::default(),
                args: Punctuated::from_iter(std::iter::once(nested_expr)),
            })
        })
}

struct MatchesMacroInput {
    expr: Expr,
    pat: Pat,
    guard: Option<(Token![if], Expr)>,
}

impl Parse for MatchesMacroInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let expr: Expr = input.parse()?;
        let _comma: Token![,] = input.parse()?;
        let pat = Pat::parse_multi_with_leading_vert(input)?;
        let guard = if input.peek(Token![if]) {
            Some((input.parse()?, input.parse()?))
        } else {
            None
        };
        Ok(Self { expr, pat, guard })
    }
}

struct AssertMacroInput {
    expr: Expr,
    trailing: Option<(Token![,], proc_macro2::TokenStream)>,
}

impl Parse for AssertMacroInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let expr: Expr = input.parse()?;
        let trailing = if input.is_empty() {
            None
        } else {
            Some((input.parse()?, input.parse()?))
        };
        Ok(Self { expr, trailing })
    }
}

struct BinaryAssertMacroInput {
    left: Expr,
    right: Expr,
    trailing: Option<(Token![,], proc_macro2::TokenStream)>,
}

impl Parse for BinaryAssertMacroInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let left: Expr = input.parse()?;
        let _comma: Token![,] = input.parse()?;
        let right: Expr = input.parse()?;
        let trailing = if input.is_empty() {
            None
        } else {
            Some((input.parse()?, input.parse()?))
        };
        Ok(Self {
            left,
            right,
            trailing,
        })
    }
}

fn macro_tail_is(path: &syn::Path, expected: &[&str]) -> bool {
    path.segments
        .last()
        .map(|segment| expected.iter().any(|name| segment.ident == *name))
        .unwrap_or(false)
}

fn is_matches_macro_path(path: &syn::Path) -> bool {
    path.is_ident("matches") || macro_tail_is(path, &["matches"])
}

fn is_assert_macro_path(path: &syn::Path) -> bool {
    path.is_ident("assert") || macro_tail_is(path, &["assert", "debug_assert"])
}

fn is_binary_assert_macro_path(path: &syn::Path) -> bool {
    macro_tail_is(
        path,
        &[
            "assert_eq",
            "assert_ne",
            "debug_assert_eq",
            "debug_assert_ne",
        ],
    )
}

fn resolve_module_path_string(
    module_path: &[syn::Ident],
    path_base: ModulePathBase,
    current_module: &str,
    span: proc_macro2::Span,
) -> Result<String, syn::Error> {
    let base = match path_base {
        ModulePathBase::Current => current_module.to_string(),
        ModulePathBase::Crate => "crate".to_string(),
        ModulePathBase::Super(depth) => ancestor_module_path(current_module, depth, span)?,
    };

    if module_path.is_empty() {
        return Ok(base);
    }

    let nested = module_path
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join("::");
    if base == "crate" {
        Ok(nested)
    } else {
        Ok(format!("{base}::{nested}"))
    }
}

fn ancestor_module_path(
    current_module: &str,
    depth: usize,
    span: proc_macro2::Span,
) -> Result<String, syn::Error> {
    if depth == 0 {
        return Ok(current_module.to_string());
    }

    if current_module == "crate" {
        return Err(syn::Error::new(
            span,
            "super:: path escapes the crate root in this module",
        ));
    }

    let mut segments = current_module
        .split("::")
        .filter(|segment| !segment.is_empty() && *segment != "crate")
        .collect::<Vec<_>>();
    if depth > segments.len() {
        return Err(syn::Error::new(
            span,
            format!(
                "super:: path climbs {depth} module levels, but the current module is only {} level(s) deep",
                segments.len()
            ),
        ));
    }

    segments.truncate(segments.len() - depth);
    if segments.is_empty() {
        Ok("crate".to_string())
    } else {
        Ok(segments.join("::"))
    }
}

fn split_module_path_base(raw_module_path: &[syn::Ident]) -> (ModulePathBase, Vec<syn::Ident>) {
    if raw_module_path.is_empty() {
        return (ModulePathBase::Current, Vec::new());
    }

    if raw_module_path[0] == "crate" {
        return (ModulePathBase::Crate, raw_module_path[1..].to_vec());
    }

    if raw_module_path[0] == "self" {
        return (ModulePathBase::Current, raw_module_path[1..].to_vec());
    }

    let mut super_depth = 0usize;
    while super_depth < raw_module_path.len() && raw_module_path[super_depth] == "super" {
        super_depth += 1;
    }
    if super_depth > 0 {
        return (
            ModulePathBase::Super(super_depth),
            raw_module_path[super_depth..].to_vec(),
        );
    }

    (ModulePathBase::Current, raw_module_path.to_vec())
}

fn resolve_enum_from_path(
    module_path: &[syn::Ident],
    path_base: ModulePathBase,
    enum_ident: &syn::Ident,
    ctx: &mut ResolveCtx<'_>,
) -> Result<Option<(ItemEnum, bool)>, syn::Error> {
    let module_path_str = resolve_module_path_string(
        module_path,
        path_base,
        ctx.current_module,
        enum_ident.span(),
    )?;
    if module_path_str != ctx.current_module {
        ensure_supported_module_path(
            &module_path_str,
            ctx.current_file,
            ctx.module_root,
            enum_ident.span(),
        )?;
    }

    if module_path_str == ctx.current_module {
        let item = ctx.enums_by_ident.get(&enum_ident.to_string()).cloned();
        if let Some(item) = item.as_ref() {
            ensure_supported_enum_shape(item, enum_ident.span())?;
        }
        let marked = item
            .as_ref()
            .map(|i| matches!(nestum_attr_kind(&i.attrs), Ok(NestumAttrKind::Empty)))
            .unwrap_or(false);
        return Ok(item.map(|i| (i, marked)));
    }

    if let Err(err) = load_module_enums(
        proc_macro2::Span::call_site(),
        &module_path_str,
        ctx.current_file,
        ctx.module_root,
        ctx.cache,
    ) {
        if matches!(path_base, ModulePathBase::Crate | ModulePathBase::Super(_)) {
            return Err(err);
        }
        return Ok(None);
    }

    let enums = match ctx.cache.get(&module_path_str) {
        Some(enums) => enums,
        None => return Ok(None),
    };

    let item = enums.get(&enum_ident.to_string()).cloned();
    if let Some(item) = item.as_ref() {
        ensure_supported_enum_shape(item, enum_ident.span())?;
    }
    let marked = item
        .as_ref()
        .map(|i| matches!(nestum_attr_kind(&i.attrs), Ok(NestumAttrKind::Empty)))
        .unwrap_or(false);
    Ok(item.map(|i| (i, marked)))
}

fn absolute_module_idents(module_path: &str) -> Vec<syn::Ident> {
    let mut segments: Vec<String> = module_path
        .split("::")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if segments.is_empty() {
        segments.push("crate".to_string());
    } else if segments[0] != "crate" {
        segments.insert(0, "crate".to_string());
    }

    segments
        .into_iter()
        .map(|s| syn::Ident::new(&s, proc_macro2::Span::call_site()))
        .collect()
}

enum NestumAttrKind {
    None,
    Empty,
    WithArgs,
}

const INVALID_VARIANT_NESTUM_USAGE: &str =
    "invalid #[nestum] on variant; use #[nestum(external = \"path::to::Enum\")]";
const INVALID_VARIANT_NESTUM_ARGS: &str =
    "invalid #[nestum(...)] on variant; expected external = \"path::to::Enum\"";

#[derive(Default, FromMeta)]
#[darling(default)]
struct NestumVariantArgs {
    external: Option<syn::LitStr>,
}

fn nestum_attr_kind(attrs: &[Attribute]) -> Result<NestumAttrKind, syn::Error> {
    for attr in attrs.iter() {
        if is_nestum_attr_path(attr.path()) {
            match &attr.meta {
                Meta::Path(_) => return Ok(NestumAttrKind::Empty),
                Meta::List(_) | Meta::NameValue(_) => return Ok(NestumAttrKind::WithArgs),
            }
        }
    }
    Ok(NestumAttrKind::None)
}

fn parse_variant_external_path(attrs: &[Attribute]) -> Result<Option<syn::Path>, syn::Error> {
    for attr in attrs.iter() {
        if !is_nestum_attr_path(attr.path()) {
            continue;
        }

        match &attr.meta {
            Meta::Path(_) => {
                return Err(syn::Error::new(attr.span(), INVALID_VARIANT_NESTUM_USAGE));
            }
            Meta::NameValue(_) => {
                return Err(syn::Error::new(attr.span(), INVALID_VARIANT_NESTUM_ARGS));
            }
            Meta::List(list) => {
                let metas = NestedMeta::parse_meta_list(list.tokens.clone())
                    .map_err(|_| syn::Error::new(attr.span(), INVALID_VARIANT_NESTUM_ARGS))?;
                if metas.is_empty() {
                    return Err(syn::Error::new(attr.span(), INVALID_VARIANT_NESTUM_USAGE));
                }

                let args = NestumVariantArgs::from_list(&metas).map_err(|_| {
                    if let Some(span) = non_string_external_value_span(&metas) {
                        return syn::Error::new(span, "external must be a string literal");
                    }
                    syn::Error::new(attr.span(), INVALID_VARIANT_NESTUM_ARGS)
                })?;

                let Some(path_str) = args.external else {
                    return Err(syn::Error::new(attr.span(), INVALID_VARIANT_NESTUM_ARGS));
                };

                let parsed: syn::Path = syn::parse_str(&path_str.value()).map_err(|_| {
                    syn::Error::new(
                        path_str.span(),
                        "external must be a valid Rust path, e.g. \"crate::foo::Enum\"",
                    )
                })?;
                return Ok(Some(parsed));
            }
        }
    }

    Ok(None)
}

fn non_string_external_value_span(metas: &[NestedMeta]) -> Option<proc_macro2::Span> {
    for meta in metas {
        let NestedMeta::Meta(meta) = meta else {
            continue;
        };
        match meta {
            Meta::NameValue(name_value) if name_value.path.is_ident("external") => {
                match &name_value.value {
                    syn::Expr::Lit(expr_lit) if matches!(expr_lit.lit, syn::Lit::Str(_)) => {}
                    other => return Some(other.span()),
                }
            }
            Meta::Path(path) if path.is_ident("external") => {
                return Some(path.span());
            }
            _ => {}
        }
    }
    None
}

fn extract_single_tuple_type(variant: &syn::Variant) -> Result<syn::Type, syn::Error> {
    match &variant.fields {
        syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            Ok(fields.unnamed.first().unwrap().ty.clone())
        }
        _ => Err(syn::Error::new(
            variant.span(),
            "nested variants must be tuple variants with exactly one field",
        )),
    }
}

fn extract_simple_ident(ty: &syn::Type) -> Result<syn::Ident, syn::Error> {
    match ty {
        syn::Type::Path(type_path)
            if type_path.qself.is_none()
                && type_path.path.segments.len() == 1
                && matches!(
                    type_path.path.segments[0].arguments,
                    syn::PathArguments::None
                ) =>
        {
            Ok(type_path.path.segments[0].ident.clone())
        }
        _ => Err(syn::Error::new(
            ty.span(),
            "nested enum type must be a simple path ident",
        )),
    }
}

fn extract_plain_enum_path(ty: &syn::Type) -> Result<Option<PlainEnumPath>, syn::Error> {
    match ty {
        syn::Type::Group(group) => extract_plain_enum_path(&group.elem),
        syn::Type::Paren(paren) => extract_plain_enum_path(&paren.elem),
        syn::Type::Path(type_path) => {
            if type_path.qself.is_some() {
                return Err(syn::Error::new(
                    ty.span(),
                    "nested enum type cannot use qualified self or associated paths; use a plain enum path like Inner or crate::inner::Inner",
                ));
            }
            if type_path.path.segments.is_empty() {
                return Ok(None);
            }

            if type_path.path.segments.len() > 1
                && type_path
                    .path
                    .segments
                    .iter()
                    .take(type_path.path.segments.len() - 1)
                    .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
            {
                return Err(syn::Error::new(
                    ty.span(),
                    "nested enum type cannot put generic arguments on module segments; use a plain enum path like foo::Inner<T>",
                ));
            }

            let last = type_path
                .path
                .segments
                .last()
                .expect("path should not be empty");
            if matches!(last.arguments, syn::PathArguments::Parenthesized(_)) {
                return Err(syn::Error::new(
                    ty.span(),
                    "nested enum type must be a plain enum path; function-style path arguments are not supported",
                ));
            }

            let enum_ident = last.ident.clone();
            let raw_module_path = type_path
                .path
                .segments
                .iter()
                .take(type_path.path.segments.len() - 1)
                .map(|segment| segment.ident.clone())
                .collect::<Vec<_>>();
            let (path_base, module_path) = split_module_path_base(&raw_module_path);

            Ok(Some(PlainEnumPath {
                path_base,
                module_path,
                enum_ident,
                enum_arguments: last.arguments.clone(),
            }))
        }
        _ => Ok(None),
    }
}

fn resolve_span_file_path(span: &proc_macro2::Span) -> Option<String> {
    let display_path = span.file();
    let display_buf = std::path::PathBuf::from(&display_path);

    if let Some(local_path) = span.local_file() {
        if local_path.is_file() {
            return Some(local_path.to_string_lossy().to_string());
        }
        if local_path.is_dir() {
            if let Some(file_name) = display_buf.file_name() {
                let candidate = local_path.join(file_name);
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
            let candidate = local_path.join(&display_buf);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    if display_path.starts_with('<') {
        return None;
    }
    if display_buf.is_absolute() {
        if display_buf.is_file() {
            return Some(display_buf.to_string_lossy().to_string());
        }
        return None;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let joined = std::path::Path::new(&manifest_dir).join(display_path);
    if joined.is_file() {
        Some(joined.to_string_lossy().to_string())
    } else {
        None
    }
}

fn normalize_non_src_module_path(file_path: &str, module_path: String) -> String {
    let Some(stem) = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
    else {
        return module_path;
    };

    if module_path == stem {
        return "crate".to_string();
    }

    let prefix = format!("{stem}::");
    module_path
        .strip_prefix(&prefix)
        .map(|rest| rest.to_string())
        .unwrap_or(module_path)
}

fn compose_scanned_module_path(base: &str, nested: &str) -> String {
    if base == "crate" || base.is_empty() {
        nested.to_string()
    } else {
        format!("{base}::{nested}")
    }
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

fn raw_string_prefix_len(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    if bytes.get(start) != Some(&b'r') {
        return None;
    }

    let mut idx = start + 1;
    let mut hashes = 0usize;
    while bytes.get(idx) == Some(&b'#') {
        hashes += 1;
        idx += 1;
    }
    if bytes.get(idx) != Some(&b'"') {
        return None;
    }

    Some((hashes, idx - start + 1))
}

fn find_module_path_by_text_scan(
    file_path: &str,
    line_number: usize,
    base: &str,
) -> Option<String> {
    #[derive(Clone, Copy)]
    enum Mode {
        Normal,
        LineComment,
        BlockComment { depth: usize },
        String { escaped: bool },
        Char { escaped: bool },
        RawString { hashes: usize },
    }

    let content = std::fs::read_to_string(file_path).ok()?;
    let bytes = content.as_bytes();
    let mut line_paths = vec![String::new()];
    let mut line = 1usize;
    let mut i = 0usize;
    let mut mode = Mode::Normal;
    let mut brace_stack: Vec<Option<String>> = Vec::new();
    let mut module_stack: Vec<String> = Vec::new();
    let mut expect_mod_ident = false;
    let mut pending_mod_name: Option<String> = None;
    let mut expect_mod_open = false;

    let current_module_path = |stack: &[String]| -> String {
        if stack.is_empty() {
            String::new()
        } else {
            stack.join("::")
        }
    };

    while i < bytes.len() {
        let byte = bytes[i];

        if byte == b'\n' {
            line += 1;
            if line_paths.len() < line {
                line_paths.push(current_module_path(&module_stack));
            } else if let Some(existing) = line_paths.get_mut(line - 1) {
                *existing = current_module_path(&module_stack);
            }
        }

        match mode {
            Mode::LineComment => {
                if byte == b'\n' {
                    mode = Mode::Normal;
                }
                i += 1;
                continue;
            }
            Mode::BlockComment { depth } => {
                if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    mode = Mode::BlockComment { depth: depth + 1 };
                    i += 2;
                    continue;
                }
                if byte == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    mode = if depth == 1 {
                        Mode::Normal
                    } else {
                        Mode::BlockComment { depth: depth - 1 }
                    };
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            Mode::String { escaped } => {
                if byte == b'\\' && !escaped {
                    mode = Mode::String { escaped: true };
                } else if byte == b'"' && !escaped {
                    mode = Mode::Normal;
                } else {
                    mode = Mode::String { escaped: false };
                }
                i += 1;
                continue;
            }
            Mode::Char { escaped } => {
                if byte == b'\\' && !escaped {
                    mode = Mode::Char { escaped: true };
                } else if byte == b'\'' && !escaped {
                    mode = Mode::Normal;
                } else {
                    mode = Mode::Char { escaped: false };
                }
                i += 1;
                continue;
            }
            Mode::RawString { hashes } => {
                if byte == b'"' {
                    let mut matched = true;
                    for offset in 0..hashes {
                        if bytes.get(i + 1 + offset) != Some(&b'#') {
                            matched = false;
                            break;
                        }
                    }
                    if matched {
                        mode = Mode::Normal;
                        i += 1 + hashes;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            Mode::Normal => {}
        }

        if byte == b'/' && bytes.get(i + 1) == Some(&b'/') {
            mode = Mode::LineComment;
            i += 2;
            continue;
        }
        if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            mode = Mode::BlockComment { depth: 1 };
            i += 2;
            continue;
        }
        if byte == b'"' {
            mode = Mode::String { escaped: false };
            i += 1;
            continue;
        }
        if byte == b'\'' {
            mode = Mode::Char { escaped: false };
            i += 1;
            continue;
        }
        if let Some((hashes, consumed)) = raw_string_prefix_len(bytes, i) {
            mode = Mode::RawString { hashes };
            i += consumed;
            continue;
        }

        if is_ident_start(byte) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &content[start..i];

            if expect_mod_ident {
                pending_mod_name = Some(ident.to_string());
                expect_mod_ident = false;
                expect_mod_open = true;
                continue;
            }

            if ident == "mod" {
                expect_mod_ident = true;
            }
            continue;
        }

        match byte {
            b'{' => {
                if expect_mod_open {
                    let module_name = pending_mod_name.take();
                    if let Some(module_name) = module_name.clone() {
                        module_stack.push(module_name);
                    }
                    brace_stack.push(module_name);
                    expect_mod_open = false;
                } else {
                    brace_stack.push(None);
                }
            }
            b';' => {
                if expect_mod_open {
                    pending_mod_name = None;
                    expect_mod_open = false;
                }
            }
            b'}' => {
                if let Some(module_name) = brace_stack.pop().flatten()
                    && module_stack.last() == Some(&module_name)
                {
                    module_stack.pop();
                }
                expect_mod_ident = false;
                expect_mod_open = false;
                pending_mod_name = None;
            }
            _ => {}
        }

        i += 1;
    }

    if line_number == 0 {
        return Some(base.to_string());
    }

    match line_paths.get(line_number.saturating_sub(1)) {
        Some(path) if !path.is_empty() => Some(compose_scanned_module_path(base, path)),
        _ => Some(base.to_string()),
    }
}

fn has_cfg_like_attrs(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        path.is_ident("cfg") || path.is_ident("cfg_attr")
    })
}

fn has_path_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("path"))
}

fn items_contain_include_macro(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Macro(item_macro) => item_macro.mac.path.is_ident("include"),
        _ => false,
    })
}

fn parse_source_file(file_path: &std::path::Path) -> Result<syn::File, syn::Error> {
    let content = std::fs::read_to_string(file_path).map_err(|err| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("failed to read source file {}: {err}", file_path.display()),
        )
    })?;

    syn::parse_file(&content).map_err(|err| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("failed to parse source file {}: {err}", file_path.display()),
        )
    })
}

fn ensure_supported_module_segments(
    items: &[Item],
    segments: &[&str],
    prefix: &mut Vec<String>,
    current_file: &str,
    module_root: &std::path::Path,
    span: proc_macro2::Span,
) -> Result<(), syn::Error> {
    let Some((segment, rest)) = segments.split_first() else {
        return Ok(());
    };

    let Some(module) = items.iter().find_map(|item| match item {
        Item::Mod(item_mod) if item_mod.ident == *segment => Some(item_mod),
        _ => None,
    }) else {
        return Ok(());
    };

    prefix.push((*segment).to_string());
    let module_path = prefix.join("::");

    if has_cfg_like_attrs(&module.attrs) {
        prefix.pop();
        return Err(syn::Error::new(
            span,
            format!(
                "nestum cannot resolve module {module_path} authoritatively because it uses #[cfg] or #[cfg_attr]; cfg-gated modules are unsupported for nesting resolution"
            ),
        ));
    }

    if has_path_attr(&module.attrs) {
        prefix.pop();
        return Err(syn::Error::new(
            span,
            format!(
                "nestum cannot resolve module {module_path} authoritatively through #[path = ...]; #[path] modules are unsupported for nesting resolution"
            ),
        ));
    }

    let result = match &module.content {
        Some((_, inner_items)) => {
            if items_contain_include_macro(inner_items) {
                Err(syn::Error::new(
                    span,
                    format!(
                        "nestum cannot resolve module {module_path} authoritatively because its body uses include!(...); include!-generated modules are unsupported for nesting resolution"
                    ),
                ))
            } else {
                ensure_supported_module_segments(
                    inner_items,
                    rest,
                    prefix,
                    current_file,
                    module_root,
                    span,
                )
            }
        }
        None => {
            let Some(module_file) = module_path_to_file(&module_path, current_file, module_root)
            else {
                prefix.pop();
                return Ok(());
            };
            let parsed = parse_source_file(&module_file)?;
            ensure_supported_module_segments(
                &parsed.items,
                rest,
                prefix,
                current_file,
                module_root,
                span,
            )
        }
    };

    prefix.pop();
    result
}

fn ensure_supported_module_path(
    module_path: &str,
    current_file: &str,
    module_root: &std::path::Path,
    span: proc_macro2::Span,
) -> Result<(), syn::Error> {
    let module_path = module_path.strip_prefix("crate::").unwrap_or(module_path);
    if module_path.is_empty() || module_path == "crate" {
        return Ok(());
    }

    let root_file = module_path_to_file("crate", current_file, module_root)
        .unwrap_or_else(|| std::path::PathBuf::from(current_file));
    let parsed = parse_source_file(&root_file)?;
    let segments = module_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    ensure_supported_module_segments(
        &parsed.items,
        &segments,
        &mut Vec::new(),
        current_file,
        module_root,
        span,
    )
}

fn ensure_supported_enum_shape(item: &ItemEnum, span: proc_macro2::Span) -> Result<(), syn::Error> {
    if has_cfg_like_attrs(&item.attrs) {
        return Err(syn::Error::new(
            span,
            format!(
                "nestum cannot resolve enum {} authoritatively because it uses #[cfg] or #[cfg_attr]; cfg-gated enums are unsupported for nesting resolution",
                item.ident
            ),
        ));
    }

    for variant in &item.variants {
        if has_cfg_like_attrs(&variant.attrs) {
            return Err(syn::Error::new(
                span,
                format!(
                    "nestum cannot resolve enum {} authoritatively because variant {} uses #[cfg] or #[cfg_attr]; cfg-gated enum variants are unsupported for nesting resolution",
                    item.ident, variant.ident
                ),
            ));
        }

        for field in variant_from_fields(variant) {
            if has_cfg_like_attrs(&field.attrs) {
                return Err(syn::Error::new(
                    span,
                    format!(
                        "nestum cannot resolve enum {} authoritatively because fields on variant {} use #[cfg] or #[cfg_attr]; cfg-gated enum fields are unsupported for nesting resolution",
                        item.ident, variant.ident
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn current_module_context(
    span: proc_macro2::Span,
) -> Result<(String, std::path::PathBuf, String), syn::Error> {
    let (file_path, fallback_line) = callsite_source_info().ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "unable to locate source file for nestum expansion; \
nestum requires proc-macro source locations to resolve nested enums authoritatively and errors instead of guessing when that context is unavailable",
        )
    })?;
    let line = match span.start().line {
        0 => fallback_line,
        line => Some(line),
    };

    let module_root = module_root_from_file(&file_path);
    let module_path = if module_root
        .to_string_lossy()
        .replace('\\', "/")
        .ends_with("/src")
    {
        let base = module_path_from_file_with_root(&file_path, &module_root);
        if let Some(line) = line {
            find_module_path_by_text_scan(&file_path, line, &base).unwrap_or(base)
        } else {
            base
        }
    } else if let Some(line) = line {
        find_module_path_by_text_scan(&file_path, line, "crate")
            .map(|path| normalize_non_src_module_path(&file_path, path))
            .unwrap_or_else(|| "crate".to_string())
    } else {
        "crate".to_string()
    };

    Ok((file_path, module_root, module_path))
}

fn load_module_enums(
    span: proc_macro2::Span,
    module_path: &str,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
) -> Result<(), syn::Error> {
    if cache.get(module_path).is_some() {
        return Ok(());
    }

    let module_file = module_path_to_file(module_path, current_file, module_root);
    let module_file = match module_file {
        Some(file) => file,
        None => {
            let all = collect_enums_by_module_path(current_file, module_root, current_file)?;
            if all.contains_key(module_path) {
                for (module, enums) in all.into_iter() {
                    cache.insert(module, enums);
                }
                return Ok(());
            }
            return Err(syn::Error::new(
                span,
                format!(
                    "unable to locate module file for {module_path}; \
expected {module_path}.rs or {module_path}/mod.rs under the module root"
                ),
            ));
        }
    };

    let all = collect_enums_by_module_path(
        module_file.to_string_lossy().as_ref(),
        module_root,
        current_file,
    )?;
    for (module, enums) in all.into_iter() {
        cache.insert(module, enums);
    }

    Ok(())
}

fn collect_enums_by_module_path(
    file_path: &str,
    module_root: &std::path::Path,
    current_file: &str,
) -> Result<ModuleEnumCache, syn::Error> {
    let content = std::fs::read_to_string(file_path).map_err(|err| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("failed to read source file: {err}"),
        )
    })?;

    let parsed = syn::parse_file(&content).map_err(|err| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("failed to parse source file: {err}"),
        )
    })?;

    let base = if file_path == current_file
        && !module_root
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("/src")
    {
        "crate".to_string()
    } else {
        module_path_from_file_with_root(file_path, module_root)
    };
    let mut map: ModuleEnumCache = HashMap::new();

    fn join_module_path(base: &str, stack: &[String]) -> String {
        if stack.is_empty() {
            return base.to_string();
        }
        let nested = stack.join("::");
        if base == "crate" {
            nested
        } else {
            format!("{base}::{nested}")
        }
    }

    fn visit_items(
        items: &[syn::Item],
        stack: &mut Vec<String>,
        base: &str,
        map: &mut ModuleEnumCache,
    ) {
        for item in items {
            match item {
                syn::Item::Enum(item_enum) => {
                    let module_path = join_module_path(base, stack);
                    map.entry(module_path)
                        .or_default()
                        .insert(item_enum.ident.to_string(), item_enum.clone());
                }
                syn::Item::Mod(module) => {
                    let Some((_, inner_items)) = &module.content else {
                        continue;
                    };
                    stack.push(module.ident.to_string());
                    visit_items(inner_items, stack, base, map);
                    stack.pop();
                }
                _ => {}
            }
        }
    }

    visit_items(&parsed.items, &mut Vec::new(), &base, &mut map);
    Ok(map)
}

fn external_path_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn is_nestum_attr_path(path: &syn::Path) -> bool {
    if path.is_ident("nestum") {
        return true;
    }
    let mut segments = path.segments.iter();
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(second) = segments.next() else {
        return false;
    };
    if segments.next().is_some() {
        return false;
    }
    first.ident == "nestum" && second.ident == "nestum"
}

// module loading handled by load_module_enums()

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variant_external_path_accepts_valid_external() {
        let attrs: Vec<Attribute> =
            vec![syn::parse_quote!(#[nestum(external = "crate::inner::Inner")])];

        let path = parse_variant_external_path(&attrs)
            .expect("external path parsing should succeed")
            .expect("external path should be present");

        assert_eq!(quote::quote!(#path).to_string(), "crate :: inner :: Inner");
    }

    #[test]
    fn parse_variant_external_path_rejects_non_string_external() {
        let attrs: Vec<Attribute> = vec![syn::parse_quote!(#[nestum(external = 123)])];

        let err = parse_variant_external_path(&attrs).expect_err("parsing should fail");
        assert_eq!(err.to_string(), "external must be a string literal");
    }

    #[test]
    fn parse_variant_external_path_rejects_unknown_keys() {
        let attrs: Vec<Attribute> = vec![syn::parse_quote!(#[nestum(foo = "bar")])];

        let err = parse_variant_external_path(&attrs).expect_err("parsing should fail");
        assert_eq!(err.to_string(), INVALID_VARIANT_NESTUM_ARGS);
    }
}
