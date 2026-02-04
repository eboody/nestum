#![feature(proc_macro_span)]

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use syn::{
    parse_macro_input, punctuated::Punctuated, spanned::Spanned, Attribute, ExprMatch, Fields,
    Item, ItemEnum, Meta, MetaNameValue, Pat, PatOr, PatPath, PatStruct, PatTupleStruct, Token,
};

#[proc_macro_attribute]
pub fn nestum(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "nestum does not accept arguments",
        )
        .to_compile_error()
        .into();
    }

    let item = parse_macro_input!(input as Item);
    match item {
        Item::Enum(item_enum) => expand_enum(item_enum)
            .unwrap_or_else(|err| err.to_compile_error())
            .into(),
        other => syn::Error::new(other.span(), "nestum can only be applied to enums")
            .to_compile_error()
            .into(),
    }
}

#[proc_macro]
pub fn nestum_match(input: TokenStream) -> TokenStream {
    let expr = parse_macro_input!(input as ExprMatch);
    expand_match(expr).unwrap_or_else(|err| err.to_compile_error()).into()
}

#[proc_macro]
pub fn nested(input: TokenStream) -> TokenStream {
    nestum_match(input)
}

fn expand_enum(item: ItemEnum) -> Result<proc_macro2::TokenStream, syn::Error> {
    let (file_path, module_root, module_path) = current_module_context()?;
    let enums_by_ident = ensure_module_enums_loaded(&module_path, &file_path, &module_root)?;

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

    expand_enum_with_context(
        item,
        &enums_by_ident,
        &marked_enums,
        &file_path,
        &module_root,
    )
}

fn expand_match(expr: ExprMatch) -> Result<proc_macro2::TokenStream, syn::Error> {
    let (file_path, module_root, module_path) = current_module_context()?;
    let enums_by_ident = ensure_module_enums_loaded(&module_path, &file_path, &module_root)?;

    let mut arms = Vec::new();
    for mut arm in expr.arms {
        arm.pat = rewrite_pat(
            arm.pat,
            &file_path,
            &module_root,
            &module_path,
            &enums_by_ident,
        )?;
        arms.push(arm);
    }

    let expr_value = expr.expr;
    Ok(quote! {
        match #expr_value {
            #(#arms),*
        }
    })
}

fn expand_enum_with_context(
    item: ItemEnum,
    enums_by_ident: &HashMap<String, ItemEnum>,
    marked_enums: &HashSet<String>,
    current_file: &str,
    module_root: &std::path::Path,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let vis = item.vis.clone();
    let enum_ident = item.ident.clone();
    let enum_mod_ident = enum_ident.clone();

    let mut enum_attrs = Vec::new();
    for attr in &item.attrs {
        if !attr.path().is_ident("nestum") {
            enum_attrs.push(attr.clone());
        }
    }

    let mut enum_variants = Vec::new();
    let mut nested_variant_modules = Vec::new();

    for variant in item.variants.iter() {
        let external_path = parse_variant_external_path(&variant.attrs)?;
        let mut cleaned_attrs = Vec::new();
        for attr in &variant.attrs {
            if !attr.path().is_ident("nestum") {
                cleaned_attrs.push(attr.clone());
            }
        }

        let mut variant_clean = variant.clone();
        variant_clean.attrs = cleaned_attrs;
        enum_variants.push(variant_clean);

        if let Some(external_path) = external_path {
            let inner_ty = extract_single_tuple_type(variant).map_err(|_| {
                syn::Error::new(
                    variant.span(),
                    format!(
                        "variant {}::{} uses #[nestum(external = \"...\")], \
but is not a single-field tuple variant",
                        enum_ident, variant.ident
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

            let external_ident = external_path
                .segments
                .last()
                .map(|s| s.ident.clone())
                .ok_or_else(|| {
                    syn::Error::new(
                        external_path.span(),
                        "external path must include an enum ident",
                    )
                })?;

            if inner_ident != external_ident {
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

            let (inner_enum, inner_is_marked) =
                resolve_external_enum(&external_path, current_file, module_root)?.ok_or_else(|| {
                    syn::Error::new(
                        external_path.span(),
                        format!(
                            "external enum {} not found; \
ensure the module path exists and the enum is declared in that module",
                            external_path_to_string(&external_path),
                        ),
                    )
                })?;

            let variant_ident = &variant.ident;
            let wrapper_items = build_wrappers_with_path(
                &enum_ident,
                variant_ident,
                &inner_enum,
                inner_is_marked,
                &external_path,
            )?;

            nested_variant_modules.push(quote! {
                pub mod #variant_ident {
                    #(#wrapper_items)*
                }
            });
        } else if let Ok(inner_ty) = extract_single_tuple_type(variant) {
            if let Ok(inner_ident) = extract_simple_ident(&inner_ty) {
                if let Some(inner_enum) = enums_by_ident.get(&inner_ident.to_string()) {
                    let inner_is_marked = marked_enums.contains(&inner_ident.to_string());
                    if inner_is_marked {
                        let variant_ident = &variant.ident;
                        let wrapper_items = build_wrappers(
                            &enum_ident,
                            variant_ident,
                            inner_enum,
                            inner_is_marked,
                        )?;

                        nested_variant_modules.push(quote! {
                            pub mod #variant_ident {
                                #(#wrapper_items)*
                            }
                        });
                    }
                } else if let Some(locations) = find_marked_enum_modules(&inner_ident)? {
                    let locations = locations.join(", ");
                    return Err(syn::Error::new(
                        inner_ty.span(),
                        format!(
                            "nested enum type {} is marked with #[nestum] in a different module ({locations}); \
use #[nestum(external = \"path::to::{}\")], or move the enum into the same module",
                            inner_ident, inner_ident
                        ),
                    ));
                }
            }
        }
    }

    Ok(quote! {
        #vis mod #enum_mod_ident {
            #(#enum_attrs)*
            #vis enum #enum_ident {
                #(#enum_variants),*
            }

            #(#nested_variant_modules)*
        }
    })
}

fn build_wrappers(
    outer_enum: &syn::Ident,
    outer_variant: &syn::Ident,
    inner_enum: &ItemEnum,
    inner_is_marked: bool,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    let inner_enum_ident = &inner_enum.ident;
    let mut items = Vec::new();
    for inner_variant in inner_enum.variants.iter() {
        let inner_ident = &inner_variant.ident;
        let inner_variant_path = if inner_is_marked {
            quote! { super::super::#inner_enum_ident::#inner_enum_ident::#inner_ident }
        } else {
            quote! { super::super::#inner_enum_ident::#inner_ident }
        };

        match &inner_variant.fields {
            Fields::Unit => {
                items.push(quote! {
                    pub const #inner_ident: super::#outer_enum =
                        super::#outer_enum::#outer_variant(#inner_variant_path);
                });
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
                items.push(quote! {
                    pub fn #inner_ident(#(#args),*) -> super::#outer_enum {
                        super::#outer_enum::#outer_variant(#inner_variant_path(#(#arg_idents),*))
                    }
                });
            }
            Fields::Named(fields) => {
                let args: Vec<_> = fields
                    .named
                    .iter()
                    .map(|f| {
                        let ident = f.ident.as_ref().unwrap();
                        let ty = &f.ty;
                        quote! { #ident: #ty }
                    })
                    .collect();
                let arg_idents: Vec<_> = fields
                    .named
                    .iter()
                    .map(|f| {
                        let ident = f.ident.as_ref().unwrap();
                        quote! { #ident }
                    })
                    .collect();
                items.push(quote! {
                    pub fn #inner_ident(#(#args),*) -> super::#outer_enum {
                        super::#outer_enum::#outer_variant(#inner_variant_path { #(#arg_idents),* })
                    }
                });
            }
        }
    }

    Ok(items)
}

fn build_wrappers_with_path(
    outer_enum: &syn::Ident,
    outer_variant: &syn::Ident,
    inner_enum: &ItemEnum,
    inner_is_marked: bool,
    inner_path: &syn::Path,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    let inner_enum_ident = &inner_enum.ident;
    let mut items = Vec::new();
    for inner_variant in inner_enum.variants.iter() {
        let inner_ident = &inner_variant.ident;
        let inner_variant_path = if inner_is_marked {
            quote! { #inner_path::#inner_enum_ident::#inner_ident }
        } else {
            quote! { #inner_path::#inner_ident }
        };

        match &inner_variant.fields {
            Fields::Unit => {
                items.push(quote! {
                    pub const #inner_ident: super::#outer_enum =
                        super::#outer_enum::#outer_variant(#inner_variant_path);
                });
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
                items.push(quote! {
                    pub fn #inner_ident(#(#args),*) -> super::#outer_enum {
                        super::#outer_enum::#outer_variant(#inner_variant_path(#(#arg_idents),*))
                    }
                });
            }
            Fields::Named(fields) => {
                let args: Vec<_> = fields
                    .named
                    .iter()
                    .map(|f| {
                        let ident = f.ident.as_ref().unwrap();
                        let ty = &f.ty;
                        quote! { #ident: #ty }
                    })
                    .collect();
                let arg_idents: Vec<_> = fields
                    .named
                    .iter()
                    .map(|f| {
                        let ident = f.ident.as_ref().unwrap();
                        quote! { #ident }
                    })
                    .collect();
                items.push(quote! {
                    pub fn #inner_ident(#(#args),*) -> super::#outer_enum {
                        super::#outer_enum::#outer_variant(#inner_variant_path { #(#arg_idents),* })
                    }
                });
            }
        }
    }

    Ok(items)
}

fn rewrite_pat(
    pat: Pat,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &HashMap<String, ItemEnum>,
) -> Result<Pat, syn::Error> {
    match pat {
        Pat::Path(pat_path) => rewrite_pat_path(
            pat_path,
            current_file,
            module_root,
            current_module,
            enums_by_ident,
        ),
        Pat::TupleStruct(pat_tuple) => rewrite_pat_tuple_struct(
            pat_tuple,
            current_file,
            module_root,
            current_module,
            enums_by_ident,
        ),
        Pat::Struct(pat_struct) => rewrite_pat_struct(
            pat_struct,
            current_file,
            module_root,
            current_module,
            enums_by_ident,
        ),
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
    enums_by_ident: &HashMap<String, ItemEnum>,
) -> Result<Pat, syn::Error> {
    let Some((module_path, explicit_crate, outer_enum, outer_variant, inner_variant)) =
        split_nested_path(&pat_path.path)?
    else {
        return Ok(Pat::Path(pat_path));
    };

    let Some(outer_info) = resolve_enum_from_path(
        &module_path,
        explicit_crate,
        &outer_enum,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
    )? else {
        return Ok(Pat::Path(pat_path));
    };

    let (outer_enum_item, outer_marked) = outer_info;
    if !outer_marked {
        return Err(syn::Error::new(
            pat_path.span(),
            format!(
                "enum {} is not marked with #[nestum]; \
only #[nestum] enums support nested match patterns",
                outer_enum
            ),
        ));
    }

    let Some((inner_enum_ident, inner_enum_path, inner_explicit_crate)) =
        resolve_inner_enum_path(&outer_enum_item, &outer_variant, enums_by_ident)?
    else {
        return Ok(Pat::Path(pat_path));
    };

    let inner_enum_info = resolve_enum_from_path(
        &inner_enum_path,
        inner_explicit_crate,
        &inner_enum_ident,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
    )?;

    let (inner_enum_item, inner_marked) = inner_enum_info.ok_or_else(|| {
        syn::Error::new(
            pat_path.span(),
            format!(
                "inner enum {} not found for {}::{}; \
ensure it is declared in the referenced module",
                inner_enum_ident, outer_enum, outer_variant
            ),
        )
    })?;
    if !inner_marked {
        return Err(syn::Error::new(
            pat_path.span(),
            format!(
                "inner enum {} is not marked with #[nestum]; \
only #[nestum] enums support nested match patterns",
                inner_enum_ident
            ),
        ));
    }

    ensure_inner_variant_exists(&inner_enum_item, &inner_variant)?;

    let outer_module_idents =
        effective_module_idents(&module_path, explicit_crate, current_module);
    let inner_module_idents = if inner_enum_path.is_empty() {
        outer_module_idents.clone()
    } else {
        effective_module_idents(&inner_enum_path, inner_explicit_crate, current_module)
    };
    let outer_variant_path =
        build_path_from_idents(outer_module_idents, &[outer_enum.clone(), outer_enum.clone(), outer_variant.clone()]);
    let inner_variant_path =
        build_path_from_idents(inner_module_idents, &[inner_enum_ident.clone(), inner_enum_ident.clone(), inner_variant]);

    let inner_pat = Pat::Path(PatPath {
        attrs: Vec::new(),
        qself: None,
        path: inner_variant_path,
    });

    Ok(Pat::TupleStruct(PatTupleStruct {
        attrs: Vec::new(),
        qself: None,
        path: outer_variant_path,
        paren_token: Default::default(),
        elems: Punctuated::from_iter(std::iter::once(inner_pat)),
    }))
}

fn rewrite_pat_tuple_struct(
    pat_tuple: PatTupleStruct,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &HashMap<String, ItemEnum>,
) -> Result<Pat, syn::Error> {
    let Some((module_path, explicit_crate, outer_enum, outer_variant, inner_variant)) =
        split_nested_path(&pat_tuple.path)?
    else {
        return Ok(Pat::TupleStruct(pat_tuple));
    };

    let Some(outer_info) = resolve_enum_from_path(
        &module_path,
        explicit_crate,
        &outer_enum,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
    )? else {
        return Ok(Pat::TupleStruct(pat_tuple));
    };

    let (outer_enum_item, outer_marked) = outer_info;
    if !outer_marked {
        return Err(syn::Error::new(
            pat_tuple.span(),
            format!(
                "enum {} is not marked with #[nestum]; \
only #[nestum] enums support nested match patterns",
                outer_enum
            ),
        ));
    }

    let Some((inner_enum_ident, inner_enum_path, inner_explicit_crate)) =
        resolve_inner_enum_path(&outer_enum_item, &outer_variant, enums_by_ident)?
    else {
        return Ok(Pat::TupleStruct(pat_tuple));
    };

    let inner_enum_info = resolve_enum_from_path(
        &inner_enum_path,
        inner_explicit_crate,
        &inner_enum_ident,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
    )?;

    let (inner_enum_item, inner_marked) = inner_enum_info.ok_or_else(|| {
        syn::Error::new(
            pat_tuple.span(),
            format!(
                "inner enum {} not found for {}::{}; \
ensure it is declared in the referenced module",
                inner_enum_ident, outer_enum, outer_variant
            ),
        )
    })?;
    if !inner_marked {
        return Err(syn::Error::new(
            pat_tuple.span(),
            format!(
                "inner enum {} is not marked with #[nestum]; \
only #[nestum] enums support nested match patterns",
                inner_enum_ident
            ),
        ));
    }

    ensure_inner_variant_exists(&inner_enum_item, &inner_variant)?;

    let outer_module_idents =
        effective_module_idents(&module_path, explicit_crate, current_module);
    let inner_module_idents = if inner_enum_path.is_empty() {
        outer_module_idents.clone()
    } else {
        effective_module_idents(&inner_enum_path, inner_explicit_crate, current_module)
    };
    let outer_variant_path =
        build_path_from_idents(outer_module_idents, &[outer_enum.clone(), outer_enum.clone(), outer_variant.clone()]);
    let inner_variant_path =
        build_path_from_idents(inner_module_idents, &[inner_enum_ident.clone(), inner_enum_ident.clone(), inner_variant]);
    let elems = pat_tuple.elems;

    let inner_pat = Pat::TupleStruct(PatTupleStruct {
        attrs: Vec::new(),
        qself: None,
        path: inner_variant_path,
        paren_token: pat_tuple.paren_token,
        elems,
    });

    Ok(Pat::TupleStruct(PatTupleStruct {
        attrs: Vec::new(),
        qself: None,
        path: outer_variant_path,
        paren_token: pat_tuple.paren_token,
        elems: Punctuated::from_iter(std::iter::once(inner_pat)),
    }))
}

fn rewrite_pat_struct(
    pat_struct: PatStruct,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &HashMap<String, ItemEnum>,
) -> Result<Pat, syn::Error> {
    let Some((module_path, explicit_crate, outer_enum, outer_variant, inner_variant)) =
        split_nested_path(&pat_struct.path)?
    else {
        return Ok(Pat::Struct(pat_struct));
    };

    let Some(outer_info) = resolve_enum_from_path(
        &module_path,
        explicit_crate,
        &outer_enum,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
    )? else {
        return Ok(Pat::Struct(pat_struct));
    };

    let (outer_enum_item, outer_marked) = outer_info;
    if !outer_marked {
        return Err(syn::Error::new(
            pat_struct.span(),
            format!(
                "enum {} is not marked with #[nestum]; \
only #[nestum] enums support nested match patterns",
                outer_enum
            ),
        ));
    }

    let Some((inner_enum_ident, inner_enum_path, inner_explicit_crate)) =
        resolve_inner_enum_path(&outer_enum_item, &outer_variant, enums_by_ident)?
    else {
        return Ok(Pat::Struct(pat_struct));
    };

    let inner_enum_info = resolve_enum_from_path(
        &inner_enum_path,
        inner_explicit_crate,
        &inner_enum_ident,
        current_file,
        module_root,
        current_module,
        enums_by_ident,
    )?;

    let (inner_enum_item, inner_marked) = inner_enum_info.ok_or_else(|| {
        syn::Error::new(
            pat_struct.span(),
            format!(
                "inner enum {} not found for {}::{}; \
ensure it is declared in the referenced module",
                inner_enum_ident, outer_enum, outer_variant
            ),
        )
    })?;
    if !inner_marked {
        return Err(syn::Error::new(
            pat_struct.span(),
            format!(
                "inner enum {} is not marked with #[nestum]; \
only #[nestum] enums support nested match patterns",
                inner_enum_ident
            ),
        ));
    }

    ensure_inner_variant_exists(&inner_enum_item, &inner_variant)?;

    let outer_module_idents =
        effective_module_idents(&module_path, explicit_crate, current_module);
    let inner_module_idents = if inner_enum_path.is_empty() {
        outer_module_idents.clone()
    } else {
        effective_module_idents(&inner_enum_path, inner_explicit_crate, current_module)
    };
    let outer_variant_path =
        build_path_from_idents(outer_module_idents, &[outer_enum.clone(), outer_enum.clone(), outer_variant.clone()]);
    let inner_variant_path =
        build_path_from_idents(inner_module_idents, &[inner_enum_ident.clone(), inner_enum_ident.clone(), inner_variant]);
    let fields = pat_struct.fields;
    let rest = pat_struct.rest;

    let inner_pat = Pat::Struct(PatStruct {
        attrs: Vec::new(),
        qself: None,
        path: inner_variant_path,
        brace_token: pat_struct.brace_token,
        fields,
        rest,
    });

    Ok(Pat::TupleStruct(PatTupleStruct {
        attrs: Vec::new(),
        qself: None,
        path: outer_variant_path,
        paren_token: Default::default(),
        elems: Punctuated::from_iter(std::iter::once(inner_pat)),
    }))
}

fn split_nested_path(
    path: &syn::Path,
) -> Result<Option<(Vec<syn::Ident>, bool, syn::Ident, syn::Ident, syn::Ident)>, syn::Error> {

    let segments: Vec<_> = path.segments.iter().map(|s| s.ident.clone()).collect();
    if segments.len() < 3 {
        return Ok(None);
    }

    let outer_idx = segments.len() - 3;
    let variant_idx = segments.len() - 2;
    let inner_idx = segments.len() - 1;
    let module_path = segments[..outer_idx].to_vec();
    let explicit_crate = module_path
        .first()
        .map(|ident| ident == "crate")
        .unwrap_or(false);
    Ok(Some((
        module_path,
        explicit_crate,
        segments[outer_idx].clone(),
        segments[variant_idx].clone(),
        segments[inner_idx].clone(),
    )))
}

fn resolve_enum_from_path(
    module_path: &[syn::Ident],
    explicit_crate: bool,
    enum_ident: &syn::Ident,
    current_file: &str,
    module_root: &std::path::Path,
    current_module: &str,
    enums_by_ident: &HashMap<String, ItemEnum>,
) -> Result<Option<(ItemEnum, bool)>, syn::Error> {
    let module_path_str = if module_path.is_empty() {
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
    };

    if module_path_str == current_module {
        let item = enums_by_ident.get(&enum_ident.to_string()).cloned();
        let marked = item
            .as_ref()
            .map(|i| matches!(nestum_attr_kind(&i.attrs), Ok(NestumAttrKind::Empty)))
            .unwrap_or(false);
        return Ok(item.map(|i| (i, marked)));
    }

    if let Err(err) = ensure_external_module_loaded(
        proc_macro2::Span::call_site(),
        &module_path_str,
        current_file,
        module_root,
    ) {
        if explicit_crate {
            return Err(err);
        }
        return Ok(None);
    }

    let registry = registry_clone();
    let enums = match registry.get(&module_path_str) {
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

fn resolve_inner_enum_path(
    outer_enum: &ItemEnum,
    outer_variant: &syn::Ident,
    enums_by_ident: &HashMap<String, ItemEnum>,
) -> Result<Option<(syn::Ident, Vec<syn::Ident>, bool)>, syn::Error> {
    let Some(variant) = outer_enum.variants.iter().find(|v| v.ident == *outer_variant) else {
        return Ok(None);
    };

    let external_path = parse_variant_external_path(&variant.attrs)?;
    if let Some(path) = external_path {
        let explicit_crate = path
            .segments
            .first()
            .map(|s| s.ident == "crate")
            .unwrap_or(false);
        let (module_path, enum_ident) = split_module_and_ident(&path).ok_or_else(|| {
            syn::Error::new(
                path.span(),
                "external path must include an enum ident, e.g. crate::foo::Enum",
            )
        })?;
        let module_idents = module_path
            .split("::")
            .filter(|s| !s.is_empty())
            .map(|s| syn::Ident::new(s, proc_macro2::Span::call_site()))
            .collect();
        return Ok(Some((
            syn::Ident::new(&enum_ident, proc_macro2::Span::call_site()),
            module_idents,
            explicit_crate,
        )));
    }

    let inner_ty = match extract_single_tuple_type(variant) {
        Ok(inner_ty) => inner_ty,
        Err(_) => return Ok(None),
    };
    let inner_ident = extract_simple_ident(&inner_ty)?;

    if enums_by_ident.get(&inner_ident.to_string()).is_none() {
        return Err(syn::Error::new(
            inner_ty.span(),
            format!(
                "inner enum {} not found for {}::{}; \
ensure it is declared in the same module or use #[nestum(external = \"path::to::{}\")]",
                inner_ident, outer_enum.ident, outer_variant, inner_ident
            ),
        ));
    }

    Ok(Some((inner_ident, Vec::new(), false)))
}

fn ensure_inner_variant_exists(
    inner_enum: &ItemEnum,
    inner_variant: &syn::Ident,
) -> Result<(), syn::Error> {
    if inner_enum
        .variants
        .iter()
        .any(|v| v.ident == *inner_variant)
    {
        Ok(())
    } else {
        Err(syn::Error::new(
            inner_variant.span(),
            format!(
                "variant {} not found on inner enum {}",
                inner_variant, inner_enum.ident
            ),
        ))
    }
}

fn build_module_path_tokens(
    module_idents: &[syn::Ident],
    enum_ident: &syn::Ident,
) -> proc_macro2::TokenStream {
    if module_idents.is_empty() {
        quote! { #enum_ident }
    } else {
        let segments = module_idents.iter();
        quote! { #(#segments)::*::#enum_ident }
    }
}

fn effective_module_idents(
    module_path: &[syn::Ident],
    explicit_crate: bool,
    current_module: &str,
) -> Vec<syn::Ident> {
    if module_path.is_empty() {
        return if current_module == "crate" {
            Vec::new()
        } else {
            module_idents_from_str(current_module)
        };
    }

    let mut segments = module_path
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let is_crate = segments.first().map(|s| s.as_str()) == Some("crate");
    if is_crate {
        segments.remove(0);
    }
    let base = segments.join("::");
    let full = if is_crate || explicit_crate || current_module == "crate" {
        base
    } else if base.is_empty() {
        current_module.to_string()
    } else {
        format!("{current_module}::{base}")
    };
    module_idents_from_str(&full)
}

fn module_idents_from_str(path: &str) -> Vec<syn::Ident> {
    if path.is_empty() || path == "crate" {
        return Vec::new();
    }
    path.split("::")
        .filter(|s| !s.is_empty())
        .map(|s| syn::Ident::new(s, proc_macro2::Span::call_site()))
        .collect()
}

fn build_path_from_idents(
    module_idents: Vec<syn::Ident>,
    tail: &[syn::Ident],
) -> syn::Path {
    let mut segments = Vec::new();
    for ident in module_idents.into_iter().chain(tail.iter().cloned()) {
        segments.push(syn::PathSegment {
            ident,
            arguments: syn::PathArguments::None,
        });
    }

    syn::Path {
        leading_colon: None,
        segments: Punctuated::from_iter(segments),
    }
}

enum NestumAttrKind {
    None,
    Empty,
    WithArgs,
}

fn nestum_attr_kind(attrs: &[Attribute]) -> Result<NestumAttrKind, syn::Error> {
    for attr in attrs.iter() {
        if attr.path().is_ident("nestum") {
            let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            if metas.is_empty() {
                return Ok(NestumAttrKind::Empty);
            }
            return Ok(NestumAttrKind::WithArgs);
        }
    }
    Ok(NestumAttrKind::None)
}

fn parse_variant_external_path(attrs: &[Attribute]) -> Result<Option<syn::Path>, syn::Error> {
    for attr in attrs.iter() {
        if !attr.path().is_ident("nestum") {
            continue;
        }

        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        if metas.is_empty() {
            return Err(syn::Error::new(
                attr.span(),
                "invalid #[nestum] on variant; use #[nestum(external = \"path::to::Enum\")]",
            ));
        }

        for meta in metas.iter() {
            if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta {
                if path.is_ident("external") {
                    let lit = match value {
                        syn::Expr::Lit(expr_lit) => expr_lit.lit.clone(),
                        _ => {
                            return Err(syn::Error::new(
                                value.span(),
                                "external must be a string literal",
                            ))
                        }
                    };
                    let path_str = match lit {
                        syn::Lit::Str(lit_str) => lit_str,
                        _ => {
                            return Err(syn::Error::new(
                                value.span(),
                                "external must be a string literal",
                            ))
                        }
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

        return Err(syn::Error::new(
            attr.span(),
            "invalid #[nestum(...)] on variant; expected external = \"path::to::Enum\"",
        ));
    }

    Ok(None)
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
        syn::Type::Path(type_path) if type_path.qself.is_none() => {
            if let Some(seg) = type_path.path.segments.last() {
                Ok(seg.ident.clone())
            } else {
                Err(syn::Error::new(ty.span(), "unsupported type path"))
            }
        }
        _ => Err(syn::Error::new(
            ty.span(),
            "nested enum type must be a simple path ident",
        )),
    }
}

fn current_module_context() -> Result<(String, std::path::PathBuf, String), syn::Error> {
    let (file_path, line) = module_path_extractor::get_source_info().ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "unable to locate source file for #[nestum]; \
this macro requires nightly and proc_macro_span support",
        )
    })?;

    let module_root = module_path_extractor::module_root_from_file(&file_path);
    let module_path =
        module_path_extractor::find_module_path_in_file(&file_path, line, &module_root)
            .ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "unable to determine module path for #[nestum]; \
ensure the enum is in a regular Rust module file (not generated or included)",
                )
            })?;

    Ok((file_path, module_root, module_path))
}

fn ensure_module_enums_loaded(
    module_path: &str,
    current_file: &str,
    module_root: &std::path::Path,
) -> Result<HashMap<String, ItemEnum>, syn::Error> {
    if let Some(found) = registry_get(module_path) {
        return Ok(found);
    }

    let all = collect_enums_by_module_path(current_file, module_root)?;
    registry_insert_all(all);

    registry_get(module_path).ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "no enums found for current module path; \
ensure the enum is defined in the same source file and module as the macro call",
        )
    })
}

fn find_marked_enum_modules(name: &syn::Ident) -> Result<Option<Vec<String>>, syn::Error> {
    let registry = registry_clone();
    let mut locations = Vec::new();
    for (module_path, enums) in registry.iter() {
        if let Some(item_enum) = enums.get(&name.to_string()) {
            match nestum_attr_kind(&item_enum.attrs)? {
                NestumAttrKind::Empty => locations.push(module_path.clone()),
                NestumAttrKind::WithArgs => {
                    return Err(syn::Error::new(
                        item_enum.span(),
                        format!(
                            "invalid #[nestum(...)] on enum {name}; \
nestum does not accept arguments. Use #[nestum] on enums only"
                        ),
                    ));
                }
                NestumAttrKind::None => {}
            }
        }
    }

    if locations.is_empty() {
        Ok(None)
    } else {
        Ok(Some(locations))
    }
}

thread_local! {
    static REGISTRY: RefCell<HashMap<String, HashMap<String, ItemEnum>>> =
        RefCell::new(HashMap::new());
}

fn registry_get(module_path: &str) -> Option<HashMap<String, ItemEnum>> {
    REGISTRY.with(|cell| cell.borrow().get(module_path).cloned())
}

fn registry_insert_all(all: HashMap<String, HashMap<String, ItemEnum>>) {
    REGISTRY.with(|cell| {
        let mut reg = cell.borrow_mut();
        for (module, enums) in all.into_iter() {
            reg.insert(module, enums);
        }
    });
}

fn registry_clone() -> HashMap<String, HashMap<String, ItemEnum>> {
    REGISTRY.with(|cell| cell.borrow().clone())
}

fn collect_enums_by_module_path(
    file_path: &str,
    module_root: &std::path::Path,
) -> Result<HashMap<String, HashMap<String, ItemEnum>>, syn::Error> {
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

    let base = module_path_extractor::module_path_from_file_with_root(file_path, module_root);
    let mut map: HashMap<String, HashMap<String, ItemEnum>> = HashMap::new();

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
        map: &mut HashMap<String, HashMap<String, ItemEnum>>,
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
                    let Some((_, inner_items)) = &module.content else { continue };
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

fn resolve_external_enum(
    path: &syn::Path,
    current_file: &str,
    module_root: &std::path::Path,
) -> Result<Option<(ItemEnum, bool)>, syn::Error> {
    let (module_path, enum_ident) = split_module_and_ident(path).ok_or_else(|| {
        syn::Error::new(
            path.span(),
            "external path must include an enum ident, e.g. crate::foo::Enum",
        )
    })?;

    ensure_external_module_loaded(path.span(), &module_path, current_file, module_root)?;

    let registry = registry_clone();
    let enums = match registry.get(&module_path) {
        Some(enums) => enums,
        None => return Ok(None),
    };

    let item_enum = match enums.get(&enum_ident) {
        Some(item_enum) => item_enum.clone(),
        None => return Ok(None),
    };

    let marked = matches!(nestum_attr_kind(&item_enum.attrs)?, NestumAttrKind::Empty);
    Ok(Some((item_enum, marked)))
}

fn split_module_and_ident(path: &syn::Path) -> Option<(String, String)> {
    let mut segments = path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>();
    if segments.first().map(|s| s.as_str()) == Some("crate") {
        segments.remove(0);
    }
    if segments.is_empty() {
        return None;
    }
    let ident = segments.pop()?;
    let module_path = if segments.is_empty() {
        "crate".to_string()
    } else {
        segments.join("::")
    };
    Some((module_path, ident))
}

fn external_path_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn ensure_external_module_loaded(
    span: proc_macro2::Span,
    module_path: &str,
    current_file: &str,
    module_root: &std::path::Path,
) -> Result<(), syn::Error> {
    if registry_get(module_path).is_some() {
        return Ok(());
    }

    let module_file = module_path_extractor::module_path_to_file(
        module_path,
        current_file,
        module_root,
    )
    .ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "unable to locate module file for {module_path}; \
expected {module_path}.rs or {module_path}/mod.rs under the module root"
            ),
        )
    })?;

    let all = collect_enums_by_module_path(
        module_file.to_string_lossy().as_ref(),
        module_root,
    )?;
    registry_insert_all(all);

    Ok(())
}
