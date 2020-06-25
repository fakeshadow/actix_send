extern crate proc_macro;

use proc_macro::TokenStream;
use std::borrow::{Borrow, BorrowMut};

use syn::{
    export::Span, punctuated::Punctuated, spanned::Spanned, token::Paren,
    AngleBracketedGenericArguments, Arm, AttrStyle, Attribute, AttributeArgs, Block, Expr,
    ExprAsync, ExprAwait, ExprBlock, ExprCall, ExprMacro, ExprMatch, ExprPath, Field, Fields,
    FieldsNamed, FieldsUnnamed, FnArg, GenericArgument, GenericParam, Generics, Ident, ImplItem,
    ImplItemMethod, ImplItemType, Item, ItemEnum, ItemImpl, ItemStruct, Lit, Local, Macro,
    MacroDelimiter, Meta, MetaNameValue, NestedMeta, Pat, PatIdent, PatTuple, PatTupleStruct,
    PatType, PatWild, Path, PathArguments, PathSegment, PredicateType, Receiver, ReturnType,
    Signature, Stmt, TraitBound, TraitBoundModifier, Type, TypeParam, TypeParamBound, TypePath,
    Variant, VisPublic, Visibility, WhereClause, WherePredicate,
};

use quote::quote;

#[proc_macro_attribute]
pub fn actor(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse(input).expect("failed to parse input");

    match item {
        Item::Struct(mut struct_item) => {
            let (args_ident, args) = collect_args(&struct_item);

            // mutate actor fields
            let (message_ident, m_param) = mutate_actor_fields(&mut struct_item);

            let ident = struct_item.ident;
            let vis = struct_item.vis;
            let semi_token = struct_item.semi_token;
            let fields = struct_item.fields;

            let (impl_gen, impl_ty, impl_where) = struct_item.generics.split_for_impl();

            // impl Actor trait for struct;
            let expended = quote! {
                    #vis struct #ident #impl_gen
                    #impl_where
                    #fields
                    #semi_token

                    impl #impl_gen #ident #impl_gen
                    #impl_where
                    {
                        pub fn create(#( #args ),*) -> #ident #impl_gen {
                            #ident {
                                #( #args_ident ),*, #message_ident: std::marker::PhantomData
                            }
                        }
                    }

                    impl #impl_gen Actor<#m_param> for #ident #impl_ty
                    #impl_where
                    {
                    }
            };

            expended.into()
        }

        _ => {
            unreachable!("Actor must be a struct");
        }
    }
}

fn mutate_actor_fields(struct_item: &mut ItemStruct) -> (Ident, TypeParam) {
    // construct ident for MESSAGE
    let m_ident = generate_message_generic_type_param_ident(struct_item);

    // transform the MESSAGE ident to type param;
    let m_param = TypeParam {
        attrs: vec![],
        ident: m_ident.clone(),
        colon_token: None,
        bounds: Default::default(),
        eq_token: None,
        default: None,
    };

    // push our type param to struct's params
    struct_item
        .generics
        .params
        .push(GenericParam::Type(m_param.clone()));

    // construct a where clause if we don't have any.
    let where_clause = struct_item
        .generics
        .where_clause
        .get_or_insert(WhereClause {
            where_token: Default::default(),
            predicates: Default::default(),
        });

    // transform the MESSAGE type ident to type;
    let m_b_type = syn::Type::Path(type_path_from_idents(vec![m_ident]));
    let mut m_p_type = PredicateType {
        lifetimes: None,
        // we would use this type later in struct field.
        bounded_ty: m_b_type.clone(),
        colon_token: Default::default(),
        bounds: Default::default(),
    };
    let mut path = Path {
        leading_colon: None,
        segments: Punctuated::new(),
    };
    path.segments.push(PathSegment {
        ident: syn::Ident::new("Message", path.span()),
        arguments: Default::default(),
    });
    m_p_type.bounds.push(TypeParamBound::Trait(TraitBound {
        paren_token: None,
        modifier: TraitBoundModifier::None,
        lifetimes: None,
        path,
    }));

    // push our MESSAGE: Message to where_clause;
    where_clause
        .predicates
        .push(WherePredicate::Type(m_p_type.clone()));

    // iter through struct fields to make sure we have a unique field name for PhantomData<MESSAGE>
    let mut phantom_field = String::from("_message");
    loop {
        let field = struct_item.fields.iter().find(|f| {
            f.ident
                .as_ref()
                .map(|i| i == phantom_field.as_str())
                .unwrap_or(false)
        });

        if field.is_some() {
            phantom_field.push_str("s");
        } else {
            break;
        }
    }

    // make phantom data type params
    let mut path = Path {
        leading_colon: None,
        segments: Default::default(),
    };

    let mut bracket = AngleBracketedGenericArguments {
        colon2_token: None,
        lt_token: Default::default(),
        args: Default::default(),
        gt_token: Default::default(),
    };

    bracket.args.push(GenericArgument::Type(m_b_type));

    path.segments.push(PathSegment {
        ident: syn::Ident::new("std", struct_item.span()),
        arguments: PathArguments::None,
    });

    path.segments.push(PathSegment {
        ident: syn::Ident::new("marker", struct_item.span()),
        arguments: PathArguments::None,
    });

    path.segments.push(PathSegment {
        ident: syn::Ident::new("PhantomData", struct_item.span()),
        arguments: PathArguments::AngleBracketed(bracket),
    });

    let ty = Type::Path(TypePath { qself: None, path });

    // construct a new field;
    let message_ident = syn::Ident::new(phantom_field.as_str(), phantom_field.span());

    match struct_item.fields.borrow_mut() {
        Fields::Named(fields) => fields.named.push(Field {
            attrs: vec![],
            vis: Visibility::Inherited,
            ident: Some(message_ident.clone()),
            colon_token: None,
            ty: ty.clone(),
        }),
        Fields::Unnamed(fields) => fields.unnamed.push(Field {
            attrs: vec![],
            vis: Visibility::Inherited,
            ident: None,
            colon_token: None,
            ty: ty.clone(),
        }),
        _ => {}
    }

    // special treat if we have a struct without fields
    if struct_item.fields.is_empty() {
        let mut named = FieldsNamed {
            brace_token: Default::default(),
            named: Default::default(),
        };

        named.named.push(Field {
            attrs: vec![],
            vis: Visibility::Inherited,
            ident: Some(message_ident.clone()),
            colon_token: None,
            ty,
        });

        struct_item.fields = Fields::Named(named);
    }

    (message_ident, m_param)
}

const PANIC: &'static str = "message(result = \"T\") must be presented in attributes.";

#[proc_macro_attribute]
pub fn message(meta: TokenStream, input: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(meta as AttributeArgs);
    let item = syn::parse_macro_input!(input as Item);

    let arg = args.first().expect(PANIC);

    let result = match arg {
        NestedMeta::Meta(meta) => {
            let _seg = meta
                .path()
                .segments
                .iter()
                .find(|s| s.ident == "result")
                .expect(PANIC);

            match meta {
                Meta::NameValue(MetaNameValue {
                    lit: Lit::Str(lit_str),
                    ..
                }) => syn::parse_str::<syn::Type>(lit_str.value().as_str()).expect(PANIC),
                _ => panic!(PANIC),
            }
        }
        _ => panic!(PANIC),
    };

    static_message(item, result)
}

#[proc_macro_attribute]
pub fn handler(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(input as Item);

    match item {
        Item::Impl(mut impl_item) => {
            // add async_trait attribute if not presented.
            let async_trait_attr = attr_from_ident_str("async_trait");

            if !impl_item.attrs.contains(&async_trait_attr) {
                impl_item.attrs.push(async_trait_attr);
            }

            // extract message's TypePath
            let msg_type_path = impl_item
                .items
                .iter()
                .find_map(|i| match i {
                    ImplItem::Method(method) => method.sig.inputs.iter().find_map(|args| {
                        if let FnArg::Typed(PatType { ty, .. }) = args {
                            if let Type::Path(path) = ty.borrow() {
                                return Some(path.clone());
                            }
                        };
                        None
                    }),
                    _ => None,
                })
                .expect("Message Type is not presented in handle method");

            // add message's type to Handler trait
            let _ = impl_item
                .trait_
                .iter_mut()
                .map(|(_, path, _)| {
                    let path_seg = path
                        .segments
                        .first_mut()
                        .map(|path_seg| {
                            if path_seg.ident.to_string().as_str() != "Handler" {
                                panic!("Handler trait is not presented");
                            }
                            path_seg
                        })
                        .expect("Handler trait has not PathSegment");

                    let mut args = AngleBracketedGenericArguments {
                        colon2_token: None,
                        lt_token: Default::default(),
                        args: Default::default(),
                        gt_token: Default::default(),
                    };

                    args.args
                        .push(GenericArgument::Type(Type::Path(msg_type_path.clone())));

                    path_seg.arguments = PathArguments::AngleBracketed(args)
                })
                .collect::<()>();

            // add or push message's type to Actor struct's type params.
            let self_ty = impl_item.self_ty.borrow_mut();

            if let Type::Path(TypePath { path, .. }) = self_ty {
                let Path { segments, .. } = path;
                let args = segments
                    .first_mut()
                    .map(|seg| &mut seg.arguments)
                    .expect("PathSegment is missing for Actor struct");

                match args {
                    PathArguments::None => {
                        let mut bracket = AngleBracketedGenericArguments {
                            colon2_token: None,
                            lt_token: Default::default(),
                            args: Default::default(),
                            gt_token: Default::default(),
                        };

                        bracket
                            .args
                            .push(GenericArgument::Type(Type::Path(msg_type_path.clone())));

                        let args_new = PathArguments::AngleBracketed(bracket);

                        *args = args_new;
                    }
                    PathArguments::AngleBracketed(ref mut bracket) => bracket
                        .args
                        .push(GenericArgument::Type(Type::Path(msg_type_path))),
                    _ => panic!("ParenthesizedGenericArguments is not supported"),
                }
            }

            let expended = quote! { #impl_item };

            expended.into()
        }
        _ => unreachable!("Handler must be a impl for actix_send::Handler trait"),
    }
}

// Take a mod contains actor/messages/actor and pack all the messages into a actor.
#[proc_macro_attribute]
pub fn actor_mod(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(input as Item);

    match item {
        Item::Mod(mut mod_item) => {
            // we are only interested in the items.
            let (_, items) = mod_item.content.as_mut().expect("mod is empty");

            // We will throw away all struct that have message attribute and collect some info.
            let mut message_params: Vec<(Ident, Generics, Type)> = Vec::new();
            // We collect attributes separately as they would apply to the final enum.
            let mut attributes: Vec<Attribute> = Vec::new();
            // We extract the actor's ident string and use it generate message enum struct ident.
            let mut actor_ident_str = String::new();

            *items = items
                .iter_mut()
                // we throw all the items that have message attribute.
                .map(|item| {
                    match item {
                        Item::Struct(struct_item) => {
                            // before we throw them we collect all the type, field and message's return type
                            // attributes other than message are collected as well.
                            if let Some(attr) = is_ident(&struct_item.attrs, "message") {
                                let test = attr
                                    .tokens
                                    .to_string()
                                    .split("=")
                                    .collect::<Vec<&str>>()
                                    .pop()
                                    .expect("#[message(result = \"T\")] is missing")
                                    .chars()
                                    .into_iter()
                                    .filter(|char| {
                                        char != &'"'
                                            && char != &'\"'
                                            && char != &')'
                                            && char != &' '
                                    })
                                    .collect::<String>();

                                let result_typ = syn::parse_str::<syn::Type>(&test)
                                    .expect(&format!("Failed parsing string: {} to type", test));

                                message_params.push((
                                    struct_item.ident.clone(),
                                    struct_item.generics.clone(),
                                    result_typ,
                                ));

                                // ToDo: We are doing extra work here and collect the message attribute too.
                                attributes.extend(struct_item.attrs.iter().cloned());

                                // remove all attribute for message type.
                                (*struct_item).attrs = vec![];
                            }

                            if let Some(_attr) = is_ident(&struct_item.attrs, "actor") {
                                actor_ident_str = struct_item.ident.to_string();
                            }
                        }
                        _ => {}
                    }

                    item.clone()
                })
                .collect::<Vec<Item>>();

            // remove all message attributes
            attributes = attributes
                .into_iter()
                .filter(|attr| {
                    attr.path
                        .segments
                        .first()
                        .map(|seg| {
                            let PathSegment { ident, .. } = seg;
                            if ident.to_string().as_str() == "message" {
                                false
                            } else {
                                true
                            }
                        })
                        .unwrap_or(true)
                })
                .collect();

            let message_enum_ident =
                Ident::new(&format!("{}Message", actor_ident_str), Span::call_site());

            // we pack the message_params into an enum.
            let mut message_enum = ItemEnum {
                attrs: attributes,
                vis: Visibility::Public(VisPublic {
                    pub_token: Default::default(),
                }),
                enum_token: Default::default(),
                ident: message_enum_ident.clone(),
                generics: Default::default(),
                brace_token: Default::default(),
                variants: Default::default(),
            };

            let result_enum_ident =
                Ident::new(&format!("{}Result", actor_ident_str), Span::call_site());

            // pack the result type into an enum too.
            let mut result_enum = ItemEnum {
                attrs: vec![],
                vis: Visibility::Public(VisPublic {
                    pub_token: Default::default(),
                }),
                enum_token: Default::default(),
                ident: result_enum_ident.clone(),
                generics: Default::default(),
                brace_token: Default::default(),
                variants: Default::default(),
            };

            // construct a type for message enum which will be used for From trait.
            let message_enum_type =
                Type::Path(type_path_from_idents(vec![message_enum_ident.clone()]));

            // ToDo: for now we ignore all generic params for message.
            for (message_ident, _generics, result_type) in message_params.iter().cloned() {
                // construct a message's type path firstly we would use it multiple times later
                let message_type_path = type_path_from_idents(vec![message_ident.clone()]);

                // construct message enum's new variant from message ident and type path
                let mut unnamed = FieldsUnnamed {
                    paren_token: Default::default(),
                    unnamed: Default::default(),
                };
                unnamed.unnamed.push(Field {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    ident: None,
                    colon_token: None,
                    ty: Type::Path(message_type_path.clone()),
                });
                message_enum.variants.push(Variant {
                    attrs: vec![],
                    ident: message_ident.clone(),
                    fields: Fields::Unnamed(unnamed),
                    discriminant: None,
                });

                // construct message result enum's new variant from message result type
                let mut unnamed = FieldsUnnamed {
                    paren_token: Default::default(),
                    unnamed: Default::default(),
                };
                unnamed.unnamed.push(Field {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    ident: None,
                    colon_token: None,
                    ty: result_type.clone(),
                });
                result_enum.variants.push(Variant {
                    attrs: vec![],
                    ident: message_ident.clone(),
                    fields: Fields::Unnamed(unnamed),
                    discriminant: None,
                });

                // impl From<Message> for ActorMessage
                // ToDo: we construct this impl item with every iteration now which is not necessary.
                let impl_item = from_trait(
                    message_type_path.clone(),
                    message_ident.clone(),
                    message_enum_ident.clone(),
                    message_enum_type.clone(),
                );

                // impl From<ActorResult> for Message::Result
                let result_enum_type =
                    Type::Path(type_path_from_idents(vec![result_enum_ident.clone()]));

                let mut path = Path {
                    leading_colon: None,
                    segments: Default::default(),
                };
                let mut bracket = AngleBracketedGenericArguments {
                    colon2_token: None,
                    lt_token: Default::default(),
                    args: Default::default(),
                    gt_token: Default::default(),
                };

                bracket
                    .args
                    .push(GenericArgument::Type(result_enum_type.clone()));

                path.segments.push(PathSegment {
                    ident: Ident::new("From", Span::call_site()),
                    arguments: PathArguments::AngleBracketed(bracket),
                });

                let mut expr_path = Path {
                    leading_colon: None,
                    segments: Default::default(),
                };

                expr_path.segments.push(PathSegment {
                    ident: result_enum_ident.clone(),
                    arguments: Default::default(),
                });

                let mut expr_call = ExprCall {
                    attrs: vec![],
                    func: Box::new(Expr::Path(ExprPath {
                        attrs: vec![],
                        qself: None,
                        path: expr_path,
                    })),
                    paren_token: Default::default(),
                    args: Default::default(),
                };

                expr_call.args.push(Expr::Path(ExprPath {
                    attrs: vec![],
                    qself: None,
                    path: path_from_ident_str("result"),
                }));

                let mut arms = Vec::new();

                let mut arm_path = Path {
                    leading_colon: None,
                    segments: Default::default(),
                };

                arm_path.segments.push(PathSegment {
                    ident: result_enum_ident.clone(),
                    arguments: Default::default(),
                });

                arm_path.segments.push(PathSegment {
                    ident: message_ident.clone(),
                    arguments: Default::default(),
                });

                let mut pat = PatTuple {
                    attrs: vec![],
                    paren_token: Default::default(),
                    elems: Default::default(),
                };

                pat.elems.push(Pat::Ident(PatIdent {
                    attrs: vec![],
                    by_ref: None,
                    mutability: None,
                    ident: Ident::new("result", Span::call_site()),
                    subpat: None,
                }));

                arms.push(Arm {
                    attrs: vec![],
                    pat: Pat::TupleStruct(PatTupleStruct {
                        attrs: vec![],
                        path: arm_path,
                        pat,
                    }),
                    guard: None,
                    fat_arrow_token: Default::default(),
                    body: Box::new(Expr::Path(ExprPath {
                        attrs: vec![],
                        qself: None,
                        path: path_from_ident_str("result"),
                    })),
                    comma: Some(Default::default()),
                });

                arms.push(Arm {
                    attrs: vec![],
                    pat: Pat::Wild(PatWild {
                        attrs: vec![],
                        underscore_token: Default::default(),
                    }),
                    guard: None,
                    fat_arrow_token: Default::default(),
                    body: Box::new(Expr::Macro(ExprMacro {
                        attrs: vec![],
                        mac: Macro {
                            path: path_from_ident_str("unreachable"),
                            bang_token: Default::default(),
                            delimiter: MacroDelimiter::Paren(Paren {
                                span: Span::call_site(),
                            }),
                            tokens: Default::default(),
                        },
                    })),
                    comma: None,
                });

                let mut method = ImplItemMethod {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    defaultness: None,
                    sig: Signature {
                        constness: None,
                        asyncness: None,
                        unsafety: None,
                        abi: None,
                        fn_token: Default::default(),
                        ident: Ident::new("from", Span::call_site()),
                        generics: Default::default(),
                        paren_token: Default::default(),
                        inputs: Default::default(),
                        variadic: None,
                        output: ReturnType::Type(Default::default(), Box::new(result_type.clone())),
                    },
                    block: Block {
                        brace_token: Default::default(),
                        stmts: vec![Stmt::Expr(Expr::Match(ExprMatch {
                            attrs: vec![],
                            match_token: Default::default(),
                            expr: Box::new(Expr::Path(ExprPath {
                                attrs: vec![],
                                qself: None,
                                path: path_from_ident_str("msg"),
                            })),
                            brace_token: Default::default(),
                            arms,
                        }))],
                    },
                };

                method.sig.inputs.push(FnArg::Typed(PatType {
                    attrs: vec![],
                    pat: Box::new(Pat::Ident(PatIdent {
                        attrs: vec![],
                        by_ref: None,
                        mutability: None,
                        ident: Ident::new("msg", Span::call_site()),
                        subpat: None,
                    })),
                    colon_token: Default::default(),
                    ty: Box::new(result_enum_type.clone()),
                }));

                let impl_item2 = Item::Impl(ItemImpl {
                    attrs: vec![],
                    defaultness: None,
                    unsafety: None,
                    impl_token: Default::default(),
                    generics: Default::default(),
                    trait_: Some((None, path, Default::default())),
                    self_ty: Box::new(result_type.clone()),
                    brace_token: Default::default(),
                    items: vec![ImplItem::Method(method)],
                });

                items.push(impl_item);
                items.push(impl_item2);
            }

            // We should impl Message trait for the message_enum
            // ToDo: We should make message trait impl a single function for both this macro and message macro
            let message_trait_impl = ItemImpl {
                attrs: vec![],
                defaultness: None,
                unsafety: None,
                impl_token: Default::default(),
                generics: Default::default(),
                trait_: Some((None, path_from_ident_str("Message"), Default::default())),
                self_ty: Box::new(message_enum_type.clone()),
                brace_token: Default::default(),
                items: vec![ImplItem::Type(ImplItemType {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    defaultness: None,
                    type_token: Default::default(),
                    ident: Ident::new("Result", Span::call_site()),
                    generics: Default::default(),
                    eq_token: Default::default(),
                    ty: Type::Path(type_path_from_idents(vec![result_enum_ident.clone()])),
                    semi_token: Default::default(),
                })],
            };

            items.push(Item::Enum(message_enum));
            items.push(Item::Enum(result_enum));
            items.push(Item::Impl(message_trait_impl));

            let handle_methods = items
                .iter()
                .filter_map(|item| {
                    let item_impl = match item {
                        Item::Impl(i) => i,
                        _ => return None,
                    };
                    let _attr = is_ident(&item_impl.attrs, "handler")?;
                    // ToDo: we should check the actor identity in future if we want to handle multiple actors in one module
                    let impl_item = item_impl.items.first()?;

                    match impl_item {
                        ImplItem::Method(method) => Some(method),
                        _ => None,
                    }
                })
                .map(|method| {
                    // We want to collect the second arg of the inputs(The message ident)
                    // We would also want to collect the statements
                    let mut args = method.sig.inputs.iter();
                    args.next();

                    let ident = args
                        .next()
                        .map(|arg| {
                            if let FnArg::Typed(pat) = arg {
                                if let Type::Path(TypePath { path, .. }) = pat.ty.as_ref() {
                                    let seg = path.segments.first()?;
                                    return Some(&seg.ident);
                                }
                            }
                            None
                        })
                        .expect("handle method must have a legit TypePath for Message type")
                        .expect("handle method must have a argument as msg: MessageType");

                    (ident.clone(), method.block.stmts.clone())
                })
                .collect::<Vec<(Ident, Vec<Stmt>)>>();

            // ToDo: We are doing extra work removing all the #[handler] impls
            *items = items
                .iter()
                .filter(|item| {
                    let item_impl = match item {
                        Item::Impl(i) => i,
                        _ => return true,
                    };
                    is_ident(&item_impl.attrs, "handler").is_none()
                })
                .cloned()
                .collect::<Vec<Item>>();

            // We generate a real handle method for ActorMessage enum and pattern match the handle async functions.
            // The return type of this handle method would be ActorMessageResult enum.
            let actor_ident = Ident::new(actor_ident_str.as_str(), Span::call_site());

            let mut inputs = Punctuated::new();
            inputs.push(FnArg::Receiver(Receiver {
                attrs: vec![],
                reference: Some(Default::default()),
                mutability: Some(Default::default()),
                self_token: Default::default(),
            }));
            inputs.push(FnArg::Typed(PatType {
                attrs: vec![],
                pat: Box::new(Pat::Ident(PatIdent {
                    attrs: vec![],
                    by_ref: None,
                    mutability: None,
                    ident: Ident::new("msg", Span::call_site()),
                    subpat: None,
                })),
                colon_token: Default::default(),
                ty: Box::new(message_enum_type.clone()),
            }));

            let mut path = Path {
                leading_colon: None,
                segments: Default::default(),
            };

            path.segments.push(PathSegment {
                ident: message_enum_ident.clone(),
                arguments: Default::default(),
            });

            // We just throw the statements of handle method for every type of message into the final handle method's enum variants.

            let arms = message_params
                .into_iter()
                .map(|(message_ident, _, _)| {
                    let mut path = path.clone();

                    path.segments.push(PathSegment {
                        ident: message_ident.clone(),
                        arguments: Default::default(),
                    });

                    let mut pat = PatTuple {
                        attrs: vec![],
                        paren_token: Default::default(),
                        elems: Default::default(),
                    };

                    pat.elems.push(Pat::Ident(PatIdent {
                        attrs: vec![],
                        by_ref: None,
                        mutability: None,
                        ident: Ident::new("msg", Span::call_site()),
                        subpat: None,
                    }));

                    let panic = format!(
                        "We can not find Handler::handle method for message type: {}",
                        &message_ident
                    );

                    let stmts = handle_methods
                        .iter()
                        .find_map(|(ident, stmts)| {
                            if ident == &message_ident {
                                Some(stmts.clone())
                            } else {
                                None
                            }
                        })
                        .expect(panic.as_str());

                    let stmt1 = Stmt::Local(Local {
                        attrs: vec![],
                        let_token: Default::default(),
                        pat: Pat::Ident(PatIdent {
                            attrs: vec![],
                            by_ref: None,
                            mutability: None,
                            ident: Ident::new("result", Span::call_site()),
                            subpat: None,
                        }),
                        init: Some((
                            Default::default(),
                            Box::new(Expr::Async(ExprAsync {
                                attrs: vec![],
                                async_token: Default::default(),
                                capture: Some(Default::default()),
                                block: Block {
                                    brace_token: Default::default(),
                                    stmts,
                                },
                            })),
                        )),
                        semi_token: Default::default(),
                    });

                    let mut path_stmt2 = Path {
                        leading_colon: None,
                        segments: Default::default(),
                    };

                    path_stmt2.segments.push(PathSegment {
                        ident: result_enum_ident.clone(),
                        arguments: PathArguments::None,
                    });

                    path_stmt2.segments.push(PathSegment {
                        ident: message_ident.clone(),
                        arguments: PathArguments::None,
                    });

                    let mut expr_call = ExprCall {
                        attrs: vec![],
                        func: Box::new(Expr::Path(ExprPath {
                            attrs: vec![],
                            qself: None,
                            path: path_stmt2,
                        })),
                        paren_token: Default::default(),
                        args: Default::default(),
                    };

                    expr_call.args.push(Expr::Await(ExprAwait {
                        attrs: vec![],
                        base: Box::new(Expr::Path(ExprPath {
                            attrs: vec![],
                            qself: None,
                            path: path_from_ident_str("result"),
                        })),
                        dot_token: Default::default(),
                        await_token: Default::default(),
                    }));

                    let stmt2 = Stmt::Expr(Expr::Call(expr_call));

                    Arm {
                        attrs: vec![],
                        pat: Pat::TupleStruct(PatTupleStruct {
                            attrs: vec![],
                            path,
                            pat,
                        }),
                        guard: None,
                        fat_arrow_token: Default::default(),
                        body: Box::new(Expr::Block(ExprBlock {
                            attrs: vec![],
                            label: None,
                            block: Block {
                                brace_token: Default::default(),
                                stmts: vec![stmt1, stmt2],
                            },
                        })),
                        comma: Some(Default::default()),
                    }
                })
                .collect();

            let handle = Item::Impl(ItemImpl {
                attrs: vec![attr_from_ident_str("handler")],
                defaultness: None,
                unsafety: None,
                impl_token: Default::default(),
                generics: Default::default(),
                trait_: Some((None, path_from_ident_str("Handler"), Default::default())),
                self_ty: Box::new(Type::Path(type_path_from_idents(vec![actor_ident]))),
                brace_token: Default::default(),
                items: vec![ImplItem::Method(ImplItemMethod {
                    attrs: vec![],
                    vis: Visibility::Inherited,
                    defaultness: None,
                    sig: Signature {
                        constness: None,
                        asyncness: Some(Default::default()),
                        unsafety: None,
                        abi: None,
                        fn_token: Default::default(),
                        ident: Ident::new("handle", Span::call_site()),
                        generics: Default::default(),
                        paren_token: Default::default(),
                        inputs,
                        variadic: None,
                        output: ReturnType::Type(
                            Default::default(),
                            Box::new(Type::Path(type_path_from_idents(vec![
                                result_enum_ident.clone()
                            ]))),
                        ),
                    },
                    block: Block {
                        brace_token: Default::default(),
                        stmts: vec![Stmt::Expr(Expr::Match(ExprMatch {
                            attrs: vec![],
                            match_token: Default::default(),
                            expr: Box::new(Expr::Path(ExprPath {
                                attrs: vec![],
                                qself: None,
                                path: path_from_ident_str("msg"),
                            })),
                            brace_token: Default::default(),
                            arms,
                        }))],
                    },
                })],
            });

            items.push(handle);

            let expand = quote! {
                #mod_item
            };

            expand.into()
        }
        _ => unreachable!("#[actor_with_messages] must be used on a mod."),
    }
}

// helper function for generating attribute.
fn attr_from_ident_str(ident_str: &str) -> Attribute {
    Attribute {
        pound_token: Default::default(),
        style: AttrStyle::Outer,
        bracket_token: Default::default(),
        path: path_from_ident_str(ident_str),
        tokens: Default::default(),
    }
}

// helper function for generating path.
fn path_from_ident_str(ident_str: &str) -> Path {
    let mut path = Path {
        leading_colon: None,
        segments: Default::default(),
    };

    path.segments.push(PathSegment {
        ident: Ident::new(ident_str, Span::call_site()),
        arguments: Default::default(),
    });

    path
}

fn type_path_from_idents(idents: Vec<Ident>) -> TypePath {
    let mut path = Path {
        leading_colon: None,
        segments: Default::default(),
    };

    for ident in idents.into_iter() {
        path.segments.push(PathSegment {
            ident,
            arguments: Default::default(),
        })
    }

    TypePath { qself: None, path }
}

fn from_trait(
    source_type_path: TypePath,
    source_ident: Ident,
    message_enum_ident: Ident,
    message_enum_type: Type,
) -> Item {
    let mut path = Path {
        leading_colon: None,
        segments: Default::default(),
    };
    let mut bracket = AngleBracketedGenericArguments {
        colon2_token: None,
        lt_token: Default::default(),
        args: Default::default(),
        gt_token: Default::default(),
    };

    bracket
        .args
        .push(GenericArgument::Type(Type::Path(source_type_path.clone())));

    path.segments.push(PathSegment {
        ident: Ident::new("From", Span::call_site()),
        arguments: PathArguments::AngleBracketed(bracket),
    });

    let mut expr_path = Path {
        leading_colon: None,
        segments: Default::default(),
    };

    expr_path.segments.push(PathSegment {
        ident: message_enum_ident.clone(),
        arguments: Default::default(),
    });

    expr_path.segments.push(PathSegment {
        ident: source_ident.clone(),
        arguments: Default::default(),
    });

    let mut expr_call = ExprCall {
        attrs: vec![],
        func: Box::new(Expr::Path(ExprPath {
            attrs: vec![],
            qself: None,
            path: expr_path,
        })),
        paren_token: Default::default(),
        args: Default::default(),
    };

    expr_call.args.push(Expr::Path(ExprPath {
        attrs: vec![],
        qself: None,
        path: path_from_ident_str("msg"),
    }));

    let mut method = ImplItemMethod {
        attrs: vec![],
        vis: Visibility::Inherited,
        defaultness: None,
        sig: Signature {
            constness: None,
            asyncness: None,
            unsafety: None,
            abi: None,
            fn_token: Default::default(),
            ident: Ident::new("from", Span::call_site()),
            generics: Default::default(),
            paren_token: Default::default(),
            inputs: Default::default(),
            variadic: None,
            output: ReturnType::Type(
                Default::default(),
                Box::new(Type::Path(type_path_from_idents(vec![message_enum_ident]))),
            ),
        },
        block: Block {
            brace_token: Default::default(),
            stmts: vec![Stmt::Expr(Expr::Call(expr_call))],
        },
    };

    method.sig.inputs.push(FnArg::Typed(PatType {
        attrs: vec![],
        pat: Box::new(Pat::Ident(PatIdent {
            attrs: vec![],
            by_ref: None,
            mutability: None,
            ident: Ident::new("msg", Span::call_site()),
            subpat: None,
        })),
        colon_token: Default::default(),
        ty: Box::new(Type::Path(source_type_path)),
    }));

    Item::Impl(ItemImpl {
        attrs: vec![],
        defaultness: None,
        unsafety: None,
        impl_token: Default::default(),
        generics: Default::default(),
        trait_: Some((None, path, Default::default())),
        self_ty: Box::new(message_enum_type),
        brace_token: Default::default(),
        items: vec![ImplItem::Method(method)],
    })
}

fn is_ident<'a>(attrs: &'a Vec<Attribute>, ident_str: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|attr| {
        attr.path
            .segments
            .first()
            .map(|seg| {
                let PathSegment { ident, .. } = seg;
                if ident.to_string().as_str() == ident_str {
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false)
    })
}

fn static_message(item: Item, result: Type) -> TokenStream {
    match item {
        Item::Struct(struct_item) => {
            let ident = &struct_item.ident;
            let (impl_gen, impl_ty, impl_where) = struct_item.generics.split_for_impl();

            let expended = quote! {
                    #struct_item

                    impl #impl_gen Message for #ident #impl_ty
                    #impl_where
                    {
                        type Result = #result;
                    }
            };

            expended.into()
        }
        Item::Enum(enum_item) => {
            let ident = &enum_item.ident;
            let (impl_gen, impl_ty, impl_where) = enum_item.generics.split_for_impl();

            let expended = quote! {
                    #enum_item

                    impl #impl_gen Message for #ident #impl_ty
                    #impl_where
                    {
                        type Result = #result;
                    }
            };

            expended.into()
        }
        _ => unreachable!("Message must be a struct"),
    }
}

// collect struct fields and construct args as well as the struct fields' indent for create function
fn collect_args(struct_item: &ItemStruct) -> (Vec<Ident>, Vec<FnArg>) {
    let mut args_ident = Vec::new();
    let args = struct_item
        .fields
        .iter()
        .filter_map(|f| {
            let ident = f.ident.as_ref()?;

            args_ident.push(ident.clone());

            let ty = f.ty.clone();
            Some(FnArg::Typed(PatType {
                attrs: vec![],
                pat: Box::new(Pat::Ident(PatIdent {
                    attrs: vec![],
                    by_ref: None,
                    mutability: None,
                    ident: ident.clone(),
                    subpat: None,
                })),
                colon_token: Default::default(),
                ty: Box::new(ty),
            }))
        })
        .collect::<Vec<FnArg>>();

    (args_ident, args)
}

// iter through the struct and generate a unique generic type ident.
fn generate_message_generic_type_param_ident(struct_item: &ItemStruct) -> Ident {
    let mut m = String::from("MESSAGE");
    loop {
        let ident = struct_item.generics.params.iter().find(|p| match *p {
            GenericParam::Type(ty) => ty.ident == m.as_str(),
            _ => false,
        });
        if ident.is_some() {
            m.push_str("S");
        } else {
            break;
        }
    }

    Ident::new(m.as_str(), struct_item.span())
}
