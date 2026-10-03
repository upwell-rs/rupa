//! Spike derive for `rupa-spike-column-kind`. Only what the spike needs:
//! `#[entity(table = "..")]`, `#[id]`, `#[column(name = "..")]`,
//! `#[column(scalar)]`, `#[column(json)]`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Data, DeriveInput, Fields, LitStr, parse_macro_input};

#[proc_macro_derive(Entity, attributes(entity, id, column))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

enum Forced {
    Infer,
    Scalar,
    Json,
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let krate = quote!(::rupa_spike_column_kind);
    let ident = &input.ident;
    let (impl_g, ty_g, where_g) = input.generics.split_for_impl();
    let generic_params: Vec<_> = input
        .generics
        .type_params()
        .map(|p| p.ident.clone())
        .collect();

    let mut table = None;
    for attr in &input.attrs {
        if attr.path().is_ident("entity") {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("table") {
                    table = Some(m.value()?.parse::<LitStr>()?);
                    Ok(())
                } else {
                    Err(m.error("unknown entity attribute"))
                }
            })?;
        }
    }
    let table = table
        .ok_or_else(|| syn::Error::new_spanned(ident, "missing #[entity(table = \"...\")]"))?;

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "Entity can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            ident,
            "Entity requires named fields",
        ));
    };

    let mut metas = Vec::new();
    let mut encodes = Vec::new();
    let mut decodes = Vec::new();
    let mut probe_fields = Vec::new();
    let mut probe_inits = Vec::new();

    for f in &fields.named {
        let fname = f.ident.as_ref().unwrap();
        let fty = &f.ty;
        let mut col_name = LitStr::new(&fname.to_string(), fname.span());
        let mut forced = Forced::Infer;
        for attr in &f.attrs {
            if attr.path().is_ident("column") {
                attr.parse_nested_meta(|m| {
                    if m.path.is_ident("name") {
                        col_name = m.value()?.parse()?;
                    } else if m.path.is_ident("scalar") {
                        forced = Forced::Scalar;
                    } else if m.path.is_ident("json") {
                        forced = Forced::Json;
                    } else {
                        return Err(m.error("unknown column attribute"));
                    }
                    Ok(())
                })?;
            }
        }

        // Autoref dispatch resolves where the code is *written*, not where it is
        // instantiated: for a field typed by a struct generic it would follow the
        // impl's bounds and silently pick a kind. Require an explicit choice.
        if matches!(forced, Forced::Infer) && mentions_any(fty, &generic_params) {
            return Err(syn::Error::new_spanned(
                fty,
                "column kind cannot be inferred for a field whose type depends on a generic parameter; \
                 add #[column(scalar)] or #[column(json)]",
            ));
        }

        let codec = match forced {
            Forced::Scalar => quote!(#krate::ScalarCodec::<#fty>::new()),
            Forced::Json => quote!(#krate::JsonCodec::<#fty>::new()),
            Forced::Infer => quote_spanned!(fty.span()=> {
                #[allow(unused_imports)]
                use #krate::__private::{JsonLevel as _, NullableJsonLevel as _, ScalarLevel as _};
                (&&&#krate::__private::Probe::<#ident #ty_g, #fty>::new(#col_name)).__rupa_codec()
            }),
        };

        metas.push(quote! {{
            let codec = #codec;
            #krate::ColumnMeta {
                field: stringify!(#fname),
                name: #col_name,
                kind: #krate::__private::kind_of::<#fty, _>(&codec),
                nullable: #krate::__private::nullable_of::<#fty, _>(&codec),
            }
        }});
        encodes.push(quote!(#krate::Codec::<#fty>::encode(&#codec, &self.#fname)));
        decodes.push(quote! {
            #fname: #krate::Codec::<#fty>::decode(
                &#codec,
                values.next().ok_or_else(|| #krate::DecodeError("missing column".into()))?,
            )?
        });
        probe_fields.push(quote!(pub #fname: #krate::__private::Probe<#ident #ty_g, #fty>));
        probe_inits.push(quote!(#fname: #krate::__private::Probe::new(#col_name)));
    }

    let fields_ty = format_ident!("__Rupa{}Fields", ident);
    Ok(quote! {
        impl #impl_g #krate::Entity for #ident #ty_g #where_g {
            const TABLE: &'static str = #table;
            fn columns() -> ::std::vec::Vec<#krate::ColumnMeta> {
                ::std::vec![#(#metas),*]
            }
            fn to_values(&self) -> ::std::vec::Vec<#krate::Value> {
                ::std::vec![#(#encodes),*]
            }
            fn from_values(values: ::std::vec::Vec<#krate::Value>) -> ::std::result::Result<Self, #krate::DecodeError> {
                let mut values = values.into_iter();
                ::std::result::Result::Ok(Self { #(#decodes),* })
            }
        }

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #fields_ty #impl_g #where_g { #(#probe_fields),* }

        impl #impl_g #ident #ty_g #where_g {
            #[doc(hidden)]
            pub const fn __rupa_fields() -> #fields_ty #ty_g {
                #fields_ty { #(#probe_inits),* }
            }
        }
    })
}

fn mentions_any(ty: &syn::Type, params: &[syn::Ident]) -> bool {
    struct V<'a>(&'a [syn::Ident], bool);
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if let Some(first) = p.segments.first()
                && self.0.contains(&first.ident)
            {
                self.1 = true;
            }
            syn::visit::visit_path(self, p);
        }
    }
    let mut v = V(params, false);
    v.visit_type(ty);
    v.1
}
