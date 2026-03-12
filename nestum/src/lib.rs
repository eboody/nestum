use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use darling::FromMeta;
use darling::ast::NestedMeta;
use module_path_extractor::{
    get_source_info as extractor_get_source_info,
    module_path_from_file_with_root, module_path_to_file, module_root_from_file,
};
use proc_macro::TokenStream;
use proc_macro_error2::proc_macro_error;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ExprBlock, ExprCall, ExprLet, ExprMacro, ExprMatch, ExprPath, ExprStruct,
    Fields, Item, ItemEnum, Local, Meta, Pat, PatOr, PatPath, PatStruct, PatTupleStruct,
    Stmt, Token, parse::{Parse, ParseStream}, parse_macro_input, punctuated::Punctuated,
    spanned::Spanned, visit_mut::{self, VisitMut},
};

type EnumMap = HashMap<String, ItemEnum>;
type ModuleEnumCache = HashMap<String, EnumMap>;

#[derive(Clone)]
struct PlainEnumPath {
    module_path: Vec<syn::Ident>,
    explicit_crate: bool,
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
use nested! for constructors, if let, while let, let-else, and matches!: {err}"
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

enum NestedInput {
    Expr(Expr),
    Stmts(Vec<Stmt>),
}

fn parse_nested_input(tokens: proc_macro2::TokenStream) -> syn::Result<NestedInput> {
    if let Ok(expr) = syn::parse2::<Expr>(tokens.clone()) {
        return Ok(NestedInput::Expr(expr));
    }

    let wrapped = quote!({ #tokens });
    let block = syn::parse2::<ExprBlock>(wrapped).map_err(|err| {
        syn::Error::new(
            err.span(),
            format!(
                "nested! expects an expression or a statement block; \
for let-else, pass statements directly inside nested! {{ ... }}: {err}"
            ),
        )
    })?;
    Ok(NestedInput::Stmts(block.block.stmts))
}

fn expand_nested_input(input: NestedInput) -> Result<proc_macro2::TokenStream, syn::Error> {
    match input {
        NestedInput::Expr(expr) => expand_nested_expr(expr),
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
    if let Some((file_path, line_number)) = extractor_get_source_info() {
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
    attrs.iter()
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
    if let Some(variant) = item.variants.iter().find(|variant| variant.ident == reserved) {
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
    let source_key = source_key_for_callsite();
    register_enum_shape(source_key.as_deref(), &item)?;

    let (file_path, module_root, module_path) = match current_module_context(item.span()) {
        Ok(ctx) => ctx,
        Err(_) => {
            let source_file = callsite_source_info()
                .map(|(path, _)| path)
                .or_else(|| source_key.clone());
            if let Some(source_file) = source_file.as_deref()
                && let Some(expanded) =
                    try_expand_enum_from_source_file(item.clone(), source_file)?
            {
                return Ok(expanded);
            }
            let enum_name = item.ident.to_string();
            let expanded =
                expand_enum_no_context(item, source_key.as_deref(), source_file.as_deref())?;
            if let Some(source_file) = source_file.as_deref() {
                record_expanded_enum(source_file, &enum_name);
            }
            return Ok(expanded);
        }
    };
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

    let enum_name = item.ident.to_string();
    let expanded = expand_enum_with_context(
        item,
        &enums_by_ident,
        &marked_enums,
        &effective_module_path,
        &file_path,
        &module_root,
        &mut cache,
    )?;
    record_expanded_enum(&file_path, &enum_name);
    Ok(expanded)
}

fn expand_match(expr: ExprMatch) -> Result<proc_macro2::TokenStream, syn::Error> {
    expand_nested_expr(Expr::Match(expr))
}

fn expand_nested_expr(expr: Expr) -> Result<proc_macro2::TokenStream, syn::Error> {
    let expr_span = expr.span();
    let (file_path, module_root, module_path) = match current_module_context(expr_span) {
        Ok(ctx) => ctx,
        Err(_) => {
            return Ok(quote! { #expr });
        }
    };
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

fn try_expand_enum_from_source_file(
    item: ItemEnum,
    source_file: &str,
) -> Result<Option<proc_macro2::TokenStream>, syn::Error> {
    let source_file = normalize_file_key(source_file);
    if !std::path::Path::new(&source_file).is_file() {
        return Ok(None);
    }

    let module_root = module_root_from_file(&source_file);
    let mut cache: ModuleEnumCache = HashMap::new();
    let all = match collect_enums_by_module_path(&source_file, &module_root, &source_file) {
        Ok(all) => all,
        Err(_) => return Ok(None),
    };
    for (module, enums) in all.into_iter() {
        cache.insert(module, enums);
    }

    let requested_module_path = module_path_from_file_with_root(&source_file, &module_root);
    let (effective_module_path, enums_by_ident) = match resolve_current_module_enums(
        &cache,
        &requested_module_path,
        &source_file,
        &module_root,
    ) {
        Some(value) => value,
        None => return Ok(None),
    };

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

    let enum_name = item.ident.to_string();
    let expanded = expand_enum_with_context(
        item,
        &enums_by_ident,
        &marked_enums,
        &effective_module_path,
        &source_file,
        &module_root,
        &mut cache,
    )?;
    record_expanded_enum(&source_file, &enum_name);
    Ok(Some(expanded))
}

fn expand_enum_no_context(
    item: ItemEnum,
    source_key: Option<&str>,
    source_file: Option<&str>,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let mut enum_variants = Vec::new();
    let mut root_variant_export_idents = Vec::new();
    let mut nested_variant_module_idents = Vec::new();
    let mut nested_variant_modules = Vec::new();
    let source_file = source_file.map(|s| s.to_string());
    let source_file = source_file.as_deref();

    for variant in item.variants.iter() {
        let mut variant_clean = variant.clone();
        variant_clean.attrs = strip_nestum_attrs(&variant.attrs);
        let mut is_marked_nested = false;

        if let Ok(inner_ty) = extract_single_tuple_type(variant) {
            let Some(inner_path) = extract_plain_enum_path(&inner_ty)? else {
                root_variant_export_idents.push(variant.ident.clone());
                enum_variants.push(variant_clean);
                continue;
            };
            let inner_ident = inner_path.enum_ident.clone();

            if let Some(inner_shape) = lookup_enum_shape(source_key, &inner_ident.to_string())
                .or_else(|| discover_enum_shape_in_manifest(&inner_ident.to_string()))
            {
                if inner_shape.marked {
                    is_marked_nested = true;
                    let enum_type_path = enum_type_path_from_plain_path(&inner_path, true);
                    rewrite_variant_type_for_nested(&mut variant_clean, enum_type_path)?;

                    let variant_ident = &variant.ident;
                    let wrapper_items = if inner_path.module_path.is_empty() {
                        build_wrappers_from_shape(&item, variant_ident, &inner_ident, &inner_shape)?
                    } else {
                        let inner_base_path = base_path_from_plain_enum_path(&inner_path);
                        build_wrappers_from_shape_with_path(
                            &item,
                            variant_ident,
                            &inner_ident,
                            &inner_shape,
                            &inner_base_path,
                        )?
                    };

                    nested_variant_module_idents.push(variant_ident.clone());
                    nested_variant_modules
                        .push(build_nested_variant_module(variant_ident, &wrapper_items));
                }
            } else if inner_path.module_path.is_empty()
                && was_expanded_enum(source_file, &inner_ident.to_string())
            {
                is_marked_nested = true;
                let enum_type_path = enum_type_path_from_plain_path(&inner_path, true);
                rewrite_variant_type_for_nested(&mut variant_clean, enum_type_path)?;
            }
        }

        if !is_marked_nested {
            root_variant_export_idents.push(variant.ident.clone());
        }
        enum_variants.push(variant_clean);
    }

    Ok(emit_expanded_enum_module(
        &item,
        enum_variants,
        &root_variant_export_idents,
        &nested_variant_module_idents,
        nested_variant_modules,
    ))
}

static EXPANDED_ENUMS: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
static ENUM_SHAPES_GLOBAL: OnceLock<Mutex<HashMap<String, EnumShape>>> = OnceLock::new();
static ENUM_SHAPES_BY_SOURCE: OnceLock<Mutex<HashMap<String, HashMap<String, EnumShape>>>> =
    OnceLock::new();

#[derive(Clone)]
struct EnumShape {
    marked: bool,
    variants: Vec<VariantShape>,
}

#[derive(Clone)]
struct VariantShape {
    ident: String,
    fields: VariantShapeFields,
}

#[derive(Clone)]
enum VariantShapeFields {
    Unit,
    Unnamed(Vec<String>),
    Named(Vec<(String, String)>),
}

fn expanded_enums_store() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    EXPANDED_ENUMS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn enum_shapes_global_store() -> &'static Mutex<HashMap<String, EnumShape>> {
    ENUM_SHAPES_GLOBAL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn enum_shapes_by_source_store() -> &'static Mutex<HashMap<String, HashMap<String, EnumShape>>> {
    ENUM_SHAPES_BY_SOURCE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_file_key(file_path: &str) -> String {
    file_path.replace('\\', "/")
}

fn source_key_for_callsite() -> Option<String> {
    let span = proc_macro2::Span::call_site();
    if let Some(resolved) = resolve_span_file_path(&span) {
        return Some(normalize_file_key(&resolved));
    }

    let display = span.file();
    if display.starts_with('<') {
        None
    } else {
        Some(normalize_file_key(&display))
    }
}

fn enum_shape_from_item(item: &ItemEnum) -> Result<EnumShape, syn::Error> {
    let marked = matches!(nestum_attr_kind(&item.attrs)?, NestumAttrKind::Empty);
    let mut variants = Vec::new();

    for variant in &item.variants {
        let fields = match &variant.fields {
            Fields::Unit => VariantShapeFields::Unit,
            Fields::Unnamed(fields) => VariantShapeFields::Unnamed(
                fields
                    .unnamed
                    .iter()
                    .map(|f| {
                        let ty = &f.ty;
                        quote!(#ty).to_string()
                    })
                    .collect(),
            ),
            Fields::Named(fields) => VariantShapeFields::Named(
                fields
                    .named
                    .iter()
                    .map(|f| {
                        let ident = f.ident.as_ref().ok_or_else(|| {
                            syn::Error::new(
                                f.span(),
                                "named field is missing an identifier in enum variant",
                            )
                        })?;
                        let ty = &f.ty;
                        Ok((ident.to_string(), quote!(#ty).to_string()))
                    })
                    .collect::<Result<Vec<_>, syn::Error>>()?,
            ),
        };

        variants.push(VariantShape {
            ident: variant.ident.to_string(),
            fields,
        });
    }

    Ok(EnumShape { marked, variants })
}

fn register_enum_shape(source_key: Option<&str>, item: &ItemEnum) -> Result<(), syn::Error> {
    let shape = enum_shape_from_item(item)?;
    let enum_name = item.ident.to_string();

    {
        let mut guard = enum_shapes_global_store()
            .lock()
            .expect("enum shape registry poisoned");
        guard.insert(enum_name.clone(), shape.clone());
    }

    if let Some(source_key) = source_key {
        let mut guard = enum_shapes_by_source_store()
            .lock()
            .expect("enum shape-by-source registry poisoned");
        guard
            .entry(normalize_file_key(source_key))
            .or_default()
            .insert(enum_name, shape);
    }

    Ok(())
}

fn lookup_enum_shape(source_key: Option<&str>, enum_ident: &str) -> Option<EnumShape> {
    if let Some(source_key) = source_key {
        let key = normalize_file_key(source_key);
        let guard = enum_shapes_by_source_store()
            .lock()
            .expect("enum shape-by-source registry poisoned");
        if let Some(shape) = guard
            .get(&key)
            .and_then(|by_ident| by_ident.get(enum_ident))
            .cloned()
        {
            return Some(shape);
        }
    }

    let guard = enum_shapes_global_store()
        .lock()
        .expect("enum shape registry poisoned");
    guard.get(enum_ident).cloned()
}

fn discover_enum_shape_in_manifest(enum_ident: &str) -> Option<EnumShape> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let src_root = std::path::Path::new(&manifest_dir).join("src");
    if !src_root.is_dir() {
        return None;
    }

    let mut stack = vec![src_root];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let parsed = match syn::parse_file(&content) {
                Ok(parsed) => parsed,
                Err(_) => continue,
            };

            let Some(item_enum) = find_enum_by_ident_in_items(&parsed.items, enum_ident) else {
                continue;
            };
            let shape = match enum_shape_from_item(&item_enum) {
                Ok(shape) => shape,
                Err(_) => continue,
            };
            let mut guard = enum_shapes_global_store()
                .lock()
                .expect("enum shape registry poisoned");
            guard.insert(enum_ident.to_string(), shape.clone());
            return Some(shape);
        }
    }

    None
}

fn find_enum_by_ident_in_items(items: &[syn::Item], enum_ident: &str) -> Option<ItemEnum> {
    for item in items {
        match item {
            syn::Item::Enum(item_enum) if item_enum.ident == enum_ident => {
                return Some(item_enum.clone());
            }
            syn::Item::Mod(module) => {
                let Some((_, inner_items)) = &module.content else {
                    continue;
                };
                if let Some(item_enum) = find_enum_by_ident_in_items(inner_items, enum_ident) {
                    return Some(item_enum);
                }
            }
            _ => {}
        }
    }
    None
}

fn record_expanded_enum(file_path: &str, enum_ident: &str) {
    let mut guard = expanded_enums_store()
        .lock()
        .expect("expanded enum store poisoned");
    guard
        .entry(normalize_file_key(file_path))
        .or_default()
        .insert(enum_ident.to_string());
}

fn was_expanded_enum(file_path: Option<&str>, enum_ident: &str) -> bool {
    let guard = expanded_enums_store()
        .lock()
        .expect("expanded enum store poisoned");
    if let Some(file_path) = file_path
        && guard
            .get(&normalize_file_key(file_path))
            .map(|set| set.contains(enum_ident))
            .unwrap_or(false)
    {
        return true;
    }
    guard.values().any(|set| set.contains(enum_ident))
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

fn emit_expanded_enum_module(
    item: &ItemEnum,
    enum_variants: Vec<syn::Variant>,
    root_variant_export_idents: &[syn::Ident],
    _nested_variant_module_idents: &[syn::Ident],
    nested_variant_modules: Vec<proc_macro2::TokenStream>,
) -> proc_macro2::TokenStream {
    let vis = &item.vis;
    let enum_mod_ident = &item.ident;
    let actual_enum_ident = format_ident!("__Nestum{}", enum_mod_ident);
    let mut rewritten_item = cleaned_enum_item(item, enum_variants);
    rewritten_item.ident = actual_enum_ident.clone();
    let alias_generics = &item.generics;
    let (_, ty_generics, _) = item.generics.split_for_impl();
    let enum_alias_ident = generated_enum_alias_ident();
    let root_variant_exports = if root_variant_export_idents.is_empty() {
        quote! {}
    } else {
        quote! {
            pub use self::#actual_enum_ident::{#(#root_variant_export_idents),*};
        }
    };

    quote! {
        #[allow(non_snake_case)]
        #vis mod #enum_mod_ident {
            #[allow(unused_imports)]
            use super::*;
            #rewritten_item
            // Keep a distinct backing enum so derive macros can target a type, not this module.
            #[allow(type_alias_bounds)]
            #vis type #enum_alias_ident #alias_generics = self::#actual_enum_ident #ty_generics;
            #root_variant_exports
            #(#nested_variant_modules)*
        }
    }
}

fn expand_enum_with_context(
    item: ItemEnum,
    _enums_by_ident: &EnumMap,
    _marked_enums: &HashSet<String>,
    module_path: &str,
    current_file: &str,
    module_root: &std::path::Path,
    cache: &mut ModuleEnumCache,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let mut enum_variants = Vec::new();
    let mut root_variant_export_idents = Vec::new();
    let mut nested_variant_module_idents = Vec::new();
    let mut nested_variant_modules = Vec::new();

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
        }

        if !is_marked_nested {
            root_variant_export_idents.push(variant.ident.clone());
        }
        enum_variants.push(variant_clean);
    }

    Ok(emit_expanded_enum_module(
        &item,
        enum_variants,
        &root_variant_export_idents,
        &nested_variant_module_idents,
        nested_variant_modules,
    ))
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
    let outer_base_path =
        enum_type_path_from_module(outer_module_path, &outer_enum.ident, &syn::PathArguments::None, false);
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
        if let Some(nested_enum) = nested_enum.as_ref().filter(|nested_enum| nested_enum.marked) {
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
        items.push(build_nested_variant_module(&inner_variant.ident, &nested_items));
    }

    Ok(items)
}

fn build_wrappers_from_shape(
    outer_enum: &ItemEnum,
    outer_variant: &syn::Ident,
    inner_enum_ident: &syn::Ident,
    inner_shape: &EnumShape,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    let inner_base_path = syn::Path {
        leading_colon: None,
        segments: Punctuated::from_iter(std::iter::once(syn::PathSegment {
            ident: inner_enum_ident.clone(),
            arguments: syn::PathArguments::None,
        })),
    };
    build_wrappers_from_shape_with_path_impl(
        outer_enum,
        outer_variant,
        inner_enum_ident,
        inner_shape,
        &inner_base_path,
        true,
    )
}

fn build_wrappers_from_shape_with_path(
    outer_enum: &ItemEnum,
    outer_variant: &syn::Ident,
    inner_enum_ident: &syn::Ident,
    inner_shape: &EnumShape,
    inner_base_path: &syn::Path,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    build_wrappers_from_shape_with_path_impl(
        outer_enum,
        outer_variant,
        inner_enum_ident,
        inner_shape,
        inner_base_path,
        false,
    )
}

fn build_wrappers_from_shape_with_path_impl(
    outer_enum: &ItemEnum,
    outer_variant: &syn::Ident,
    _inner_enum_ident: &syn::Ident,
    inner_shape: &EnumShape,
    inner_base_path: &syn::Path,
    from_parent_scope: bool,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    let mut items = Vec::new();

    for inner_variant in &inner_shape.variants {
        let inner_ident = syn::parse_str::<syn::Ident>(&inner_variant.ident).map_err(|err| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "failed to parse nested variant identifier `{}`: {err}",
                    inner_variant.ident
                ),
            )
        })?;

        let inner_variant_path = if from_parent_scope {
            if inner_shape.marked {
                quote! { super::super::#inner_base_path::Enum::#inner_ident }
            } else {
                quote! { super::super::#inner_base_path::#inner_ident }
            }
        } else if inner_shape.marked {
            quote! { #inner_base_path::Enum::#inner_ident }
        } else {
            quote! { #inner_base_path::#inner_ident }
        };

        items.push(build_wrapper_from_shape_variant(
            outer_enum,
            outer_variant,
            &inner_ident,
            &inner_variant.fields,
            inner_variant_path,
        )?);
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
                    let ident = f.ident.as_ref().expect("named field should have an identifier");
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                })
                .collect();
            let arg_idents: Vec<_> = fields
                .named
                .iter()
                .map(|f| {
                    let ident = f.ident.as_ref().expect("named field should have an identifier");
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

fn build_wrapper_from_shape_variant(
    outer_enum: &ItemEnum,
    outer_variant: &syn::Ident,
    inner_ident: &syn::Ident,
    fields: &VariantShapeFields,
    inner_variant_path: proc_macro2::TokenStream,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let (fn_generics, outer_return_ty, where_clause, can_use_const) =
        wrapper_signature_tokens(outer_enum);

    match fields {
        VariantShapeFields::Unit if can_use_const => Ok(quote! {
            pub const #inner_ident: #outer_return_ty =
                super::Enum::#outer_variant(#inner_variant_path);
        }),
        VariantShapeFields::Unit => Ok(quote! {
            pub fn #inner_ident #fn_generics () -> #outer_return_ty #where_clause {
                super::Enum::#outer_variant(#inner_variant_path)
            }
        }),
        VariantShapeFields::Unnamed(types) => {
            let args: Vec<_> = types
                .iter()
                .enumerate()
                .map(|(i, ty_str)| {
                    let ident = format_ident!("v{i}");
                    let ty = syn::parse_str::<syn::Type>(ty_str).map_err(|err| {
                        syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("failed to parse nested tuple field type `{ty_str}`: {err}"),
                        )
                    })?;
                    Ok(quote! { #ident: #ty })
                })
                .collect::<Result<Vec<_>, syn::Error>>()?;

            let arg_idents: Vec<_> = types
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let ident = format_ident!("v{i}");
                    quote! { #ident }
                })
                .collect();

            Ok(quote! {
                pub fn #inner_ident #fn_generics (#(#args),*) -> #outer_return_ty #where_clause {
                    super::Enum::#outer_variant(#inner_variant_path(#(#arg_idents),*))
                }
            })
        }
        VariantShapeFields::Named(named_fields) => {
            let args: Vec<_> = named_fields
                .iter()
                .map(|(name, ty_str)| {
                    let ident = syn::parse_str::<syn::Ident>(name).map_err(|err| {
                        syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("failed to parse nested named field identifier `{name}`: {err}"),
                        )
                    })?;
                    let ty = syn::parse_str::<syn::Type>(ty_str).map_err(|err| {
                        syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("failed to parse nested named field type `{ty_str}`: {err}"),
                        )
                    })?;
                    Ok(quote! { #ident: #ty })
                })
                .collect::<Result<Vec<_>, syn::Error>>()?;

            let arg_idents: Vec<_> = named_fields
                .iter()
                .map(|(name, _)| {
                    let ident = syn::parse_str::<syn::Ident>(name).map_err(|err| {
                        syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("failed to parse nested named field identifier `{name}`: {err}"),
                        )
                    })?;
                    Ok(quote! { #ident })
                })
                .collect::<Result<Vec<_>, syn::Error>>()?;

            Ok(quote! {
                pub fn #inner_ident #fn_generics (#(#args),*) -> #outer_return_ty #where_clause {
                    super::Enum::#outer_variant(#inner_variant_path { #(#arg_idents),* })
                }
            })
        }
    }
}

fn wrapper_signature_tokens(
    outer_enum: &ItemEnum,
) -> (
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    bool,
) {
    let (_, ty_generics, _) = outer_enum.generics.split_for_impl();
    wrapper_signature_tokens_with_return_ty(outer_enum, quote! { super::Enum #ty_generics })
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

fn base_path_from_plain_enum_path(inner_path: &PlainEnumPath) -> syn::Path {
    let mut segments = inner_path
        .module_path
        .iter()
        .map(|ident| path_segment(ident, syn::PathArguments::None))
        .collect::<Vec<_>>();
    segments.push(path_segment(
        &inner_path.enum_ident,
        syn::PathArguments::None,
    ));
    syn::Path {
        leading_colon: None,
        segments: Punctuated::from_iter(segments),
    }
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

fn enum_type_path_from_plain_path(inner_path: &PlainEnumPath, marked: bool) -> syn::Path {
    let mut segments = inner_path
        .module_path
        .iter()
        .map(|ident| path_segment(ident, syn::PathArguments::None))
        .collect::<Vec<_>>();
    append_enum_type_segments(
        &mut segments,
        &inner_path.enum_ident,
        &inner_path.enum_arguments,
        marked,
    );
    syn::Path {
        leading_colon: None,
        segments: Punctuated::from_iter(segments),
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

fn outer_return_ty_tokens(outer_enum: &ItemEnum, outer_base_path: &syn::Path) -> proc_macro2::TokenStream {
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
            external_parts.explicit_crate,
            owner_module_path,
        );
        let Some((item, marked)) = resolve_enum_from_path(
            &external_parts.module_path,
            external_parts.explicit_crate,
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
        inner_path.explicit_crate,
        owner_module_path,
    );
    let Some((item, marked)) = resolve_enum_from_path(
        &inner_path.module_path,
        inner_path.explicit_crate,
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
            field.pat = Box::new(rewrite_pat(
                (*field.pat).clone(),
                current_file,
                module_root,
                current_module,
                enums_by_ident,
                cache,
            )?);
        }
        return Ok(Pat::Struct(pat_struct));
    }

    let mut fields = pat_struct.fields.clone();
    for field in &mut fields {
        field.pat = Box::new(rewrite_pat(
            (*field.pat).clone(),
            current_file,
            module_root,
            current_module,
            enums_by_ident,
            cache,
        )?);
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
        return Ok(Pat::Struct(PatStruct { fields, ..pat_struct }));
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
    explicit_crate: bool,
    enum_ident: &syn::Ident,
    ctx: &PathResolveCtx<'_>,
    cache: &mut ModuleEnumCache,
) -> Result<Option<ResolvedEnumInfo>, syn::Error> {
    let module_path_str = resolve_module_path_string(module_path, explicit_crate, ctx.current_module);
    let mut resolve_ctx = ResolveCtx {
        current_file: ctx.current_file,
        module_root: ctx.module_root,
        current_module: ctx.current_module,
        enums_by_ident: ctx.enums_by_ident,
        cache,
    };
    let Some((item, marked)) =
        resolve_enum_from_path(module_path, explicit_crate, enum_ident, &mut resolve_ctx)?
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
        let module_path = segments[..outer_idx].to_vec();
        let explicit_crate = module_path
            .first()
            .map(|ident| ident == "crate")
            .unwrap_or(false);
        let outer_enum = &segments[outer_idx];
        let variant_chain = &segments[outer_idx + 1..];

        let Some(mut current_enum) = resolve_enum_info_from_path(
            &module_path,
            explicit_crate,
            outer_enum,
            &path_ctx,
            cache,
        )
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
                    format!("variant {} not found on enum {}", variant_ident, current_enum.item.ident),
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
                return Err(syn::Error::new(
                    path.span(),
                    format!(
                        "variant {} on enum {} does not wrap a #[nestum] enum",
                        variant.ident, current_enum.item.ident
                    ),
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
            Ok(pat) => expr_let.pat = Box::new(pat),
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

    fn visit_expr_macro_mut(&mut self, expr_macro: &mut ExprMacro) {
        if self.error.is_some() {
            return;
        }

        if !is_matches_macro_path(&expr_macro.mac.path) {
            visit_mut::visit_expr_macro_mut(self, expr_macro);
            return;
        }

        let mut input = match syn::parse2::<MatchesMacroInput>(expr_macro.mac.tokens.clone()) {
            Ok(input) => input,
            Err(err) => {
                self.set_error(syn::Error::new(
                    expr_macro.mac.span(),
                    format!(
                        "unsupported matches! syntax inside nested!: {err}; \
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
        let guard = input.guard.as_ref().map(|(if_token, guard)| quote!(#if_token #guard));
        expr_macro.mac.tokens = quote!(#expr, #pat #guard).into();
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
        Ok(Self {
            expr,
            pat,
            guard,
        })
    }
}

fn is_matches_macro_path(path: &syn::Path) -> bool {
    path.is_ident("matches")
        || path
            .segments
            .last()
            .map(|segment| segment.ident == "matches")
            .unwrap_or(false)
}

fn resolve_module_path_string(
    module_path: &[syn::Ident],
    explicit_crate: bool,
    current_module: &str,
) -> String {
    if module_path.is_empty() {
        current_module.to_string()
    } else {
        let mut segments = module_path
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let is_crate = segments.first().map(|s| s.as_str()) == Some("crate");
        if is_crate {
            segments.remove(0);
        }
        let base = if segments.is_empty() {
            "crate".to_string()
        } else {
            segments.join("::")
        };
        if is_crate || current_module == "crate" || explicit_crate {
            base
        } else {
            format!("{current_module}::{base}")
        }
    }
}

fn resolve_enum_from_path(
    module_path: &[syn::Ident],
    explicit_crate: bool,
    enum_ident: &syn::Ident,
    ctx: &mut ResolveCtx<'_>,
) -> Result<Option<(ItemEnum, bool)>, syn::Error> {
    let module_path_str = resolve_module_path_string(module_path, explicit_crate, ctx.current_module);

    if module_path_str == ctx.current_module {
        let item = ctx.enums_by_ident.get(&enum_ident.to_string()).cloned();
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
        if explicit_crate {
            return Err(err);
        }
        return Ok(None);
    }

    let enums = match ctx.cache.get(&module_path_str) {
        Some(enums) => enums,
        None => return Ok(None),
    };

    let item = enums.get(&enum_ident.to_string()).cloned();
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
                && matches!(type_path.path.segments[0].arguments, syn::PathArguments::None) =>
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

            let first = &type_path.path.segments[0].ident;
            if first == "self" {
                return Err(syn::Error::new(
                    ty.span(),
                    "nested enum type cannot use self:: paths; use the bare enum name in the same module",
                ));
            }
            if first == "super" {
                return Err(syn::Error::new(
                    ty.span(),
                    "nested enum type cannot use super:: paths; use crate::path::Enum or #[nestum(external = \"crate::path::Enum\")]",
                ));
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

            let last = type_path.path.segments.last().expect("path should not be empty");
            if matches!(last.arguments, syn::PathArguments::Parenthesized(_)) {
                return Err(syn::Error::new(
                    ty.span(),
                    "nested enum type must be a plain enum path; function-style path arguments are not supported",
                ));
            }

            let enum_ident = last.ident.clone();
            let module_path = type_path
                .path
                .segments
                .iter()
                .take(type_path.path.segments.len() - 1)
                .map(|segment| segment.ident.clone())
                .collect::<Vec<_>>();
            let explicit_crate = module_path
                .first()
                .map(|ident| ident == "crate")
                .unwrap_or(false);

            Ok(Some(PlainEnumPath {
                module_path,
                explicit_crate,
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

fn find_module_path_by_text_scan(file_path: &str, line_number: usize, base: &str) -> Option<String> {
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

fn current_module_context(
    span: proc_macro2::Span,
) -> Result<(String, std::path::PathBuf, String), syn::Error> {
    let (file_path, fallback_line) = callsite_source_info().ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "unable to locate source file for #[nestum]; \
ensure proc-macro source locations are available in this build environment",
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

    fn pretty(tokens: proc_macro2::TokenStream) -> String {
        let file: syn::File =
            syn::parse2(tokens).expect("expanded tokens should parse as a complete file");
        prettyplease::unparse(&file)
    }

    #[test]
    fn no_context_fallback_snapshot_for_nested_wrappers() {
        let inner: ItemEnum = syn::parse_quote! {
            #[nestum]
            pub enum Inner {
                A,
                B(u8),
            }
        };

        register_enum_shape(None, &inner).expect("shape registration should succeed");

        let outer: ItemEnum = syn::parse_quote! {
            #[nestum]
            pub enum Outer {
                Wrap(Inner),
                Other,
            }
        };

        let expanded =
            expand_enum_no_context(outer, None, None).expect("no-context expansion should succeed");
        let pretty_expanded = pretty(expanded);

        let expected: syn::File = syn::parse_quote! {
            #[allow(non_snake_case)]
            pub mod Outer {
                #[allow(unused_imports)]
                use super::*;
                #[doc(hidden)]
                pub enum __NestumOuter {
                    Wrap(Inner::Enum),
                    Other,
                }

                #[allow(type_alias_bounds)]
                pub type Enum = self::__NestumOuter;

                pub use self::__NestumOuter::Other;

                #[allow(non_snake_case)]
                pub mod Wrap {
                    #[allow(unused_imports)]
                    use super::*;

                    pub const A: super::Enum = super::Enum::Wrap(super::super::Inner::Enum::A);

                    pub fn B(v0: u8) -> super::Enum {
                        super::Enum::Wrap(super::super::Inner::Enum::B(v0))
                    }
                }
            }
        };
        let pretty_expected = prettyplease::unparse(&expected);

        assert_eq!(pretty_expanded, pretty_expected);
    }

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

    #[test]
    fn source_file_fallback_generates_wrappers_without_registry_order() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("nestum_source_fallback_{unique}.rs"));
        let source = r#"
#[nestum]
pub enum VariantFromFile {
    Main(Color),
    Darker(Color),
    Lighter(Color),
    Lightest(Color),
}

#[nestum]
pub enum ThemeFromFile {
    Teal(VariantFromFile),
    Pink(VariantFromFile),
}
"#;

        std::fs::write(&path, source).expect("fixture source file should be writable");

        let theme: ItemEnum = syn::parse_quote! {
            #[nestum]
            pub enum ThemeFromFile {
                Teal(VariantFromFile),
                Pink(VariantFromFile),
            }
        };

        let expanded = try_expand_enum_from_source_file(
            theme,
            path.to_str()
                .expect("fixture file path should be valid utf-8"),
        )
        .expect("source fallback expansion should succeed")
        .expect("source fallback should produce expanded tokens");
        let pretty_expanded = pretty(expanded);

        assert!(pretty_expanded.contains("pub mod ThemeFromFile"));
        assert!(pretty_expanded.contains("pub mod Pink"));
        assert!(
            pretty_expanded.contains("pub fn Main(v0: Color) -> crate::ThemeFromFile::Enum")
        );
        assert!(pretty_expanded.contains("crate::VariantFromFile::Enum"));

        let _ = std::fs::remove_file(&path);
    }
}
