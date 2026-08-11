use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::{
    Attribute, Data, DeriveInput, Fields, Ident, ItemEnum, LitInt, PathArguments, Token, Type,
    ext::IdentExt,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

struct PacketArguments {
    id: Option<LitInt>,
    payload: Option<Type>,
}

impl Parse for PacketArguments {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut arguments = Self {
            id: None,
            payload: None,
        };

        while !input.is_empty() {
            let key: Ident = input.call(Ident::parse_any)?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "id" => {
                    if arguments.id.is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate packet ID"));
                    }
                    arguments.id = Some(input.parse()?);
                }
                "payload" | "type" => {
                    if arguments.payload.is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate packet payload type"));
                    }
                    arguments.payload = Some(input.parse()?);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "expected `id = ...` or `payload = ...`",
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected a comma"));
            }
        }

        Ok(arguments)
    }
}

struct VariantInfo {
    variant: Ident,
    payload: Type,
    id: LitInt,
}

fn packet_arguments(attrs: &[Attribute]) -> syn::Result<PacketArguments> {
    attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("packet"))
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing #[packet(...)]"))?
        .parse_args()
}

fn parse_variant(variant: &syn::Variant) -> syn::Result<VariantInfo> {
    let arguments = packet_arguments(&variant.attrs)?;
    let Some(id) = arguments.id else {
        return Err(syn::Error::new_spanned(
            &variant.ident,
            "missing `id = ...` in packet attribute",
        ));
    };

    let Fields::Unnamed(fields) = &variant.fields else {
        return Err(syn::Error::new_spanned(
            &variant.fields,
            "packet variants must contain one boxed payload",
        ));
    };

    if fields.unnamed.len() != 1 {
        return Err(syn::Error::new_spanned(
            fields,
            "packet variants must contain one boxed payload",
        ));
    }

    let Type::Path(path) = &fields.unnamed[0].ty else {
        return Err(syn::Error::new_spanned(
            &fields.unnamed[0].ty,
            "packet payload must be Box<PacketType>",
        ));
    };

    let segment = path.path.segments.last().unwrap();
    if segment.ident != "Box" {
        return Err(syn::Error::new_spanned(
            &fields.unnamed[0].ty,
            "packet payload must be boxed",
        ));
    }

    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            &fields.unnamed[0].ty,
            "packet payload must be Box<PacketType>",
        ));
    };

    let Some(syn::GenericArgument::Type(payload)) = arguments.args.first() else {
        return Err(syn::Error::new_spanned(
            &fields.unnamed[0].ty,
            "packet payload must be Box<PacketType>",
        ));
    };

    Ok(VariantInfo {
        variant: variant.ident.clone(),
        payload: payload.clone(),
        id,
    })
}

fn numeric_id(literal: &LitInt) -> syn::Result<u16> {
    let text = literal.to_string().replace('_', "");
    let (digits, radix) = if let Some(value) = text.strip_prefix("0x") {
        (value, 16)
    } else if let Some(value) = text.strip_prefix("0o") {
        (value, 8)
    } else if let Some(value) = text.strip_prefix("0b") {
        (value, 2)
    } else {
        (text.as_str(), 10)
    };

    u16::from_str_radix(digits, radix).map_err(|error| {
        syn::Error::new(
            literal.span(),
            format!("packet ID must fit in u16: {error}"),
        )
    })
}

#[proc_macro_attribute]
pub fn packet_enum(_attributes: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemEnum);

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(input.generics, "generic packet enums are not supported")
            .to_compile_error()
            .into();
    }

    let mut variants = Vec::new();
    for variant in &input.variants {
        let arguments = match packet_arguments(&variant.attrs) {
            Ok(arguments) => arguments,
            Err(error) => return error.to_compile_error().into(),
        };
        let Some(id) = arguments.id else {
            return syn::Error::new_spanned(
                &variant.ident,
                "missing `id = ...` in packet attribute",
            )
            .to_compile_error()
            .into();
        };
        let Some(payload) = arguments.payload else {
            return syn::Error::new_spanned(
                &variant.ident,
                "missing `payload = ...` in packet attribute",
            )
            .to_compile_error()
            .into();
        };

        if !matches!(variant.fields, Fields::Unit) {
            return syn::Error::new_spanned(
                &variant.fields,
                "packet_enum variants must be unit variants",
            )
            .to_compile_error()
            .into();
        }

        let attrs = variant
            .attrs
            .iter()
            .filter(|attribute| !attribute.path().is_ident("packet"));
        let name = &variant.ident;

        variants.push(quote! {
            #(#attrs)*
            #[packet(id = #id)]
            #name(::std::boxed::Box<#payload>)
        });
    }

    let attrs = input.attrs;
    let visibility = input.vis;
    let ident = input.ident;

    quote! {
        #(#attrs)*
        #[derive(::coeus::common::proto::Packet)]
        #visibility enum #ident {
            #(#variants),*
        }
    }
    .into()
}

#[proc_macro_derive(Packet, attributes(packet))]
pub fn derive_packet(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;

    let Data::Enum(data) = input.data else {
        return syn::Error::new_spanned(ident, "Packet can only be derived for an enum")
            .to_compile_error()
            .into();
    };

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(input.generics, "generic packet enums are not supported")
            .to_compile_error()
            .into();
    }

    let variants = match data
        .variants
        .iter()
        .map(parse_variant)
        .collect::<syn::Result<Vec<_>>>()
    {
        Ok(variants) => variants,
        Err(error) => return error.to_compile_error().into(),
    };

    let mut ids = HashSet::new();
    for variant in &variants {
        let id = match numeric_id(&variant.id) {
            Ok(id) => id,
            Err(error) => return error.to_compile_error().into(),
        };

        if !ids.insert(id) {
            return syn::Error::new_spanned(&variant.id, format!("duplicate packet ID: {id}"))
                .to_compile_error()
                .into();
        }
    }

    let wire = format_ident!("__{}_Wire", ident);

    let serialize_arms = variants.iter().map(|variant| {
        let name = &variant.variant;
        let id = &variant.id;
        quote! {
            Self::#name(value) => {
                let payload = ::postcard::to_stdvec(value.as_ref())
                    .map_err(|error| {
                        <S::Error as ::serde::ser::Error>::custom(error.to_string())
                    })?;
                (#id as u16, payload)
            }
        }
    });

    let deserialize_arms = variants.iter().map(|variant| {
        let name = &variant.variant;
        let payload = &variant.payload;
        let id = &variant.id;
        quote! {
            #id => {
                let value: #payload = ::postcard::from_bytes(&wire.payload)
                    .map_err(|error| {
                        <D::Error as ::serde::de::Error>::custom(error.to_string())
                    })?;
                Ok(Self::#name(Box::new(value)))
            }
        }
    });

    let kind_arms = variants.iter().map(|variant| {
        let name = &variant.variant;
        let id = &variant.id;
        quote! { Self::#name(_) => #id }
    });

    let expanded = quote! {
        #[allow(non_camel_case_types)]
        #[derive(::serde::Deserialize)]
        struct #wire {
            kind: u16,
            payload: Vec<u8>,
        }

        impl #ident {
            pub fn kind(&self) -> u16 {
                match self {
                    #(#kind_arms),*
                }
            }
        }

        impl ::serde::Serialize for #ident {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                use ::serde::ser::SerializeStruct;

                let (kind, payload) = match self {
                    #(#serialize_arms),*
                };

                let mut state = serializer.serialize_struct(stringify!(#ident), 2)?;
                state.serialize_field("kind", &kind)?;
                state.serialize_field("payload", &payload)?;
                state.end()
            }
        }

        impl<'de> ::serde::Deserialize<'de> for #ident {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                let wire = #wire::deserialize(deserializer)?;

                match wire.kind {
                    #(#deserialize_arms),*,
                    kind => Err(<D::Error as ::serde::de::Error>::custom(
                        format!("unknown packet kind: {kind}"),
                    )),
                }
            }
        }
    };

    expanded.into()
}
