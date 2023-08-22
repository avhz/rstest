use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    visit_mut::VisitMut,
    FnArg, Ident, ItemFn, LitStr, Token,
};

use self::files_args::{extract_files_args, ValueListFromFiles};

use super::{
    arguments::ArgumentsInfo,
    check_timeout_attrs, extract_case_args, extract_cases, extract_excluded_trace,
    extract_fixtures, extract_value_list,
    future::{extract_futures, extract_global_awt},
    parse_vector_trailing_till_double_comma,
    sys::SysEngine,
    testcase::TestCase,
    Attribute, Attributes, ExtendWithFunctionAttrs, Fixture,
};
use crate::{error::attribute_used_more_than_once, parse::vlist::ValueList};
use crate::{
    error::ErrorsVec,
    refident::{MaybeIdent, RefIdent},
    utils::attr_is,
};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, ToTokens};

pub(crate) mod files_args;

#[derive(PartialEq, Debug, Default)]
pub(crate) struct RsTestInfo {
    pub(crate) data: RsTestData,
    pub(crate) attributes: RsTestAttributes,
    pub(crate) arguments: ArgumentsInfo,
}

impl Parse for RsTestInfo {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(if input.is_empty() {
            Default::default()
        } else {
            Self {
                data: input.parse()?,
                attributes: input
                    .parse::<Token![::]>()
                    .or_else(|_| Ok(Default::default()))
                    .and_then(|_| input.parse())?,
                arguments: Default::default(),
            }
        })
    }
}

impl ExtendWithFunctionAttrs for RsTestInfo {
    fn extend_with_function_attrs<S: SysEngine>(
        &mut self,
        item_fn: &mut ItemFn,
    ) -> Result<(), ErrorsVec> {
        let composed_tuple!(_inner, excluded, _timeout, futures, global_awt) = merge_errors!(
            self.data.extend_with_function_attrs::<S>(item_fn),
            extract_excluded_trace(item_fn),
            check_timeout_attrs(item_fn),
            extract_futures(item_fn),
            extract_global_awt(item_fn)
        )?;
        self.attributes.add_notraces(excluded);
        self.arguments.set_global_await(global_awt);
        self.arguments.set_futures(futures.into_iter());
        Ok(())
    }
}

#[derive(PartialEq, Debug, Default)]
pub(crate) struct RsTestData {
    pub(crate) items: Vec<RsTestItem>,
}

impl RsTestData {
    pub(crate) fn case_args(&self) -> impl Iterator<Item = &Ident> {
        self.items.iter().filter_map(|it| match it {
            RsTestItem::CaseArgName(ref arg) => Some(arg),
            _ => None,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn has_case_args(&self) -> bool {
        self.case_args().next().is_some()
    }

    pub(crate) fn cases(&self) -> impl Iterator<Item = &TestCase> {
        self.items.iter().filter_map(|it| match it {
            RsTestItem::TestCase(ref case) => Some(case),
            _ => None,
        })
    }

    pub(crate) fn has_cases(&self) -> bool {
        self.cases().next().is_some()
    }

    pub(crate) fn fixtures(&self) -> impl Iterator<Item = &Fixture> {
        self.items.iter().filter_map(|it| match it {
            RsTestItem::Fixture(ref fixture) => Some(fixture),
            _ => None,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn has_fixtures(&self) -> bool {
        self.fixtures().next().is_some()
    }

    pub(crate) fn list_values(&self) -> impl Iterator<Item = &ValueList> {
        self.items.iter().filter_map(|mv| match mv {
            RsTestItem::ValueList(ref value_list) => Some(value_list),
            _ => None,
        })
    }

    pub(crate) fn has_list_values(&self) -> bool {
        self.list_values().next().is_some()
    }

    fn files(&self) -> Option<&Files> {
        self.items.iter().find_map(|it| match it {
            RsTestItem::Files(ref files) => Some(files),
            _ => None,
        })
    }
}

#[derive(PartialEq, Debug)]
pub(crate) struct Files {
    hierarchy: Folder,
    data: Vec<Ident>,
    args: Vec<StructField>,
}

impl Files {
    pub(crate) fn hierarchy(&self) -> &Folder {
        &self.hierarchy
    }

    pub(crate) fn data(&self) -> &[Ident] {
        self.data.as_ref()
    }

    pub(crate) fn args(&self) -> &[StructField] {
        self.args.as_ref()
    }
}

impl ToTokens for Files {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.data.iter().for_each(|data| data.to_tokens(tokens));
        self.args.iter().for_each(|f| f.to_tokens(tokens));
    }
}

impl From<Folder> for Files {
    fn from(hierarchy: Folder) -> Self {
        Self {
            hierarchy,
            data: Default::default(),
            args: Default::default(),
        }
    }
}

#[derive(PartialEq, Debug)]
pub(crate) struct Folder {
    name: String,
    files: Vec<String>,
    folders: Vec<Folder>,
}

impl Folder {
    #[cfg(test)]
    pub(crate) fn fake() -> Self {
        Self {
            name: "fake".to_string(),
            files: vec!["foo".to_string(), "bar".to_string()],
            folders: vec![Folder {
                name: "baz".to_string(),
                files: vec![],
                folders: vec![],
            }],
        }
    }

    #[cfg(test)]
    fn build_hierarchy(_path: LitStr) -> syn::Result<Self> {
        Ok(Self::fake())
    }

    #[cfg(not(test))]
    fn build_hierarchy(_path: LitStr) -> syn::Result<Self> {
        todo!("Not implemented yet")
    }
}

#[derive(PartialEq, Eq, Debug, Hash, Clone)]
pub(crate) struct StructField {
    ident: Ident,
    field: Option<String>,
}

impl StructField {
    pub(crate) fn new(ident: Ident, field: Option<String>) -> Self {
        Self { ident, field }
    }
}

impl ToTokens for StructField {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.ident.to_tokens(tokens);
    }
}

impl Parse for RsTestData {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![::]) {
            Ok(Default::default())
        } else {
            Ok(Self {
                items: parse_vector_trailing_till_double_comma::<_, Token![,]>(input)?,
            })
        }
    }
}

/// Simple struct used to visit function attributes and extract files and
/// eventually parsing errors
#[derive(Default)]
struct FilesExtractor(Option<Files>, Vec<syn::Error>);

impl VisitMut for FilesExtractor {
    fn visit_item_fn_mut(&mut self, node: &mut ItemFn) {
        let attrs = std::mem::take(&mut node.attrs);
        let mut attrs_buffer = Vec::<syn::Attribute>::default();
        for attr in attrs.into_iter() {
            if attr_is(&attr, "json") {
                match attr
                    .parse_args::<LitStr>()
                    .and_then(Folder::build_hierarchy)
                {
                    Ok(hierarchy) => {
                        self.0 = Some(hierarchy.into());
                    }
                    Err(err) => self.1.push(err),
                };
            } else {
                attrs_buffer.push(attr)
            }
        }
        node.attrs = std::mem::take(&mut attrs_buffer);
        syn::visit_mut::visit_item_fn_mut(self, node)
    }

    fn visit_fn_arg_mut(&mut self, node: &mut FnArg) {
        let (name, node) = match (node.maybe_ident().cloned(), node) {
            (Some(name), FnArg::Typed(node)) => (name, node),
            _ => {
                return;
            }
        };
        let (field, mut errors) = maybe_parse_attribute_args_just_once::<LitStr>(node, "field");
        if let Some(field) = field {
            if let Some(files) = self.0.as_mut() {
                files
                    .args
                    .push(StructField::new(name.clone(), field.map(|l| l.value())));
            } else {
                self.1.push(syn::Error::new(
                    name.span(),
                    format!("`field` attribute must be used on files test set"),
                ))
            }
        }
        self.1.append(&mut errors);
        let (attr, mut errors) = attribute_args_once(node, "data");
        if let Some(attr) = attr {
            if let Some(files) = self.0.as_mut() {
                files.data.push(name.clone());
            } else {
                self.1.push(syn::Error::new(
                    attr.span(),
                    format!("`data` attribute must be used on files test set"),
                ))
            }
        }
        self.1.append(&mut errors);
    }
}

fn maybe_parse_attribute_args_just_once<T: Parse>(
    node: &syn::PatType,
    name: &str,
) -> (Option<Option<T>>, Vec<syn::Error>) {
    let mut errors = Vec::new();
    let val = node
        .attrs
        .iter()
        .filter(|&a| attr_is(a, name))
        .map(|a| {
            (
                a,
                match &a.meta {
                    syn::Meta::Path(_path) => None,
                    _ => Some(a.parse_args::<T>()),
                },
            )
        })
        .fold(None, |first, (a, res)| match (first, res) {
            (None, None) => Some(None),
            (None, Some(Ok(parsed))) => Some(Some(parsed)),
            (first, Some(Err(err))) => {
                errors.push(err);
                first
            }
            (first, _) => {
                errors.push(attribute_used_more_than_once(a, name));
                first
            }
        });
    (val, errors)
}

fn attribute_args_once<'a>(
    node: &'a syn::PatType,
    name: &str,
) -> (Option<&'a syn::Attribute>, Vec<syn::Error>) {
    let mut errors = Vec::new();
    let mut attributes = node
        .attrs
        .iter()
        .filter(|&a| attr_is(a, name))
        .map(|a| match a.meta.require_path_only() {
            Ok(_) => a,
            Err(err) => {
                errors.push(err);
                a
            }
        })
        .collect::<Vec<_>>()
        .into_iter();
    let val = attributes.next();
    while let Some(attr) = attributes.next() {
        errors.push(attribute_used_more_than_once(attr, name));
    }
    (val, errors)
}

pub(crate) fn extract_files(item_fn: &mut ItemFn) -> Result<Option<Files>, ErrorsVec> {
    let mut extractor = FilesExtractor::default();
    extractor.visit_item_fn_mut(item_fn);

    if extractor.1.is_empty() {
        Ok(extractor.0)
    } else {
        Err(extractor.1.into())
    }
}

impl ExtendWithFunctionAttrs for RsTestData {
    fn extend_with_function_attrs<S: SysEngine>(
        &mut self,
        item_fn: &mut ItemFn,
    ) -> Result<(), ErrorsVec> {
        let composed_tuple!(fixtures, case_args, cases, value_list, files, files_args) = merge_errors!(
            extract_fixtures(item_fn),
            extract_case_args(item_fn),
            extract_cases(item_fn),
            extract_value_list(item_fn),
            extract_files(item_fn),
            extract_files_args(item_fn)
        )?;

        self.items.extend(fixtures.into_iter().map(|f| f.into()));
        self.items.extend(case_args.into_iter().map(|f| f.into()));
        self.items.extend(cases.into_iter().map(|f| f.into()));
        self.items.extend(value_list.into_iter().map(|f| f.into()));
        self.items.extend(files.into_iter().map(|f| f.into()));
        self.items.extend(
            ValueListFromFiles::default()
                .to_value_list(files_args)?
                .into_iter()
                .map(|f| f.into()),
        );
        Ok(())
    }
}

#[derive(PartialEq, Debug)]
pub(crate) enum RsTestItem {
    Fixture(Fixture),
    CaseArgName(Ident),
    TestCase(TestCase),
    ValueList(ValueList),
    Files(Files),
}

impl From<Fixture> for RsTestItem {
    fn from(f: Fixture) -> Self {
        RsTestItem::Fixture(f)
    }
}

impl From<Ident> for RsTestItem {
    fn from(ident: Ident) -> Self {
        RsTestItem::CaseArgName(ident)
    }
}

impl From<TestCase> for RsTestItem {
    fn from(case: TestCase) -> Self {
        RsTestItem::TestCase(case)
    }
}

impl From<ValueList> for RsTestItem {
    fn from(value_list: ValueList) -> Self {
        RsTestItem::ValueList(value_list)
    }
}

impl From<Files> for RsTestItem {
    fn from(value: Files) -> Self {
        RsTestItem::Files(value)
    }
}

impl Parse for RsTestItem {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.fork().parse::<TestCase>().is_ok() {
            input.parse::<TestCase>().map(RsTestItem::TestCase)
        } else if input.peek2(Token![=>]) {
            input.parse::<ValueList>().map(RsTestItem::ValueList)
        } else if input.fork().parse::<Fixture>().is_ok() {
            input.parse::<Fixture>().map(RsTestItem::Fixture)
        } else if input.fork().parse::<Ident>().is_ok() {
            input.parse::<Ident>().map(RsTestItem::CaseArgName)
        } else {
            Err(syn::Error::new(Span::call_site(), "Cannot parse it"))
        }
    }
}

impl MaybeIdent for RsTestItem {
    fn maybe_ident(&self) -> Option<&Ident> {
        use RsTestItem::*;
        match self {
            Fixture(ref fixture) => Some(fixture.ident()),
            CaseArgName(ref case_arg) => Some(case_arg),
            ValueList(ref value_list) => Some(value_list.ident()),
            TestCase(_) => None,
            Files(_) => None,
        }
    }
}

impl ToTokens for RsTestItem {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        use RsTestItem::*;
        match self {
            Fixture(ref fixture) => fixture.to_tokens(tokens),
            CaseArgName(ref case_arg) => case_arg.to_tokens(tokens),
            TestCase(ref case) => case.to_tokens(tokens),
            ValueList(ref list) => list.to_tokens(tokens),
            Files(files) => files.to_tokens(tokens),
        }
    }
}

wrap_attributes!(RsTestAttributes);

impl RsTestAttributes {
    const TRACE_VARIABLE_ATTR: &'static str = "trace";
    const NOTRACE_VARIABLE_ATTR: &'static str = "notrace";

    pub(crate) fn trace_me(&self, ident: &Ident) -> bool {
        if self.should_trace() {
            !self.iter().any(|m| Self::is_notrace(ident, m))
        } else {
            false
        }
    }

    fn is_notrace(ident: &Ident, m: &Attribute) -> bool {
        match m {
            Attribute::Tagged(i, args) if i == Self::NOTRACE_VARIABLE_ATTR => {
                args.iter().any(|a| a == ident)
            }
            _ => false,
        }
    }

    pub(crate) fn should_trace(&self) -> bool {
        self.iter().any(Self::is_trace)
    }

    pub(crate) fn add_trace(&mut self, trace: Ident) {
        self.inner.attributes.push(Attribute::Attr(trace));
    }

    pub(crate) fn add_notraces(&mut self, notraces: Vec<Ident>) {
        if notraces.is_empty() {
            return;
        }
        self.inner.attributes.push(Attribute::Tagged(
            format_ident!("{}", Self::NOTRACE_VARIABLE_ATTR),
            notraces,
        ));
    }

    fn is_trace(m: &Attribute) -> bool {
        matches!(m, Attribute::Attr(i) if i == Self::TRACE_VARIABLE_ATTR)
    }
}

impl Parse for RsTestAttributes {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(input.parse::<Attributes>()?.into())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        parse::sys::DefaultSysEngine,
        test::{assert_eq, *},
    };

    mod parse_rstest_data {
        use super::assert_eq;
        use super::*;

        fn parse_rstest_data<S: AsRef<str>>(fixtures: S) -> RsTestData {
            parse_meta(fixtures)
        }

        #[test]
        fn one_arg() {
            let fixtures = parse_rstest_data("my_fixture(42)");

            let expected = RsTestData {
                items: vec![fixture("my_fixture", &["42"]).into()],
            };

            assert_eq!(expected, fixtures);
        }
    }

    #[test]
    fn should_check_all_timeout_to_catch_the_right_errors() {
        let mut item_fn = r#"
            #[timeout(<some>)]
            #[timeout(42)]
            #[timeout]
            #[timeout(Duration::from_millis(20))]
            fn test_fn(#[case] arg: u32) {
            }
        "#
        .ast();

        let mut info = RsTestInfo::default();

        let errors = info
            .extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
            .unwrap_err();

        assert_eq!(2, errors.len());
    }

    #[cfg(feature = "async-timeout")]
    #[test]
    fn should_parse_async_timeout() {
        let mut item_fn = r#"
            #[timeout(Duration::from_millis(20))]
            async fn test_fn(#[case] arg: u32) {
            }
        "#
        .ast();

        let mut info = RsTestInfo::default();

        info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
            .unwrap();
    }

    #[cfg(not(feature = "async-timeout"))]
    #[test]
    fn should_return_error_for_async_timeout() {
        let mut item_fn = r#"
            #[timeout(Duration::from_millis(20))]
            async fn test_fn(#[case] arg: u32) {
            }
        "#
        .ast();

        let mut info = RsTestInfo::default();

        let errors = info
            .extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
            .unwrap_err();

        assert_eq!(1, errors.len());
        assert!(format!("{:?}", errors).contains("async-timeout feature"))
    }

    fn parse_rstest<S: AsRef<str>>(rstest_data: S) -> RsTestInfo {
        parse_meta(rstest_data)
    }

    mod no_cases {
        use super::{assert_eq, *};
        use crate::parse::{Attribute, Attributes};

        #[test]
        fn happy_path() {
            let data = parse_rstest(
                r#"my_fixture(42, "other"), other(vec![42])
            :: trace :: no_trace(some)"#,
            );

            let expected = RsTestInfo {
                data: vec![
                    fixture("my_fixture", &["42", r#""other""#]).into(),
                    fixture("other", &["vec![42]"]).into(),
                ]
                .into(),
                attributes: Attributes {
                    attributes: vec![
                        Attribute::attr("trace"),
                        Attribute::tagged("no_trace", vec!["some"]),
                    ],
                }
                .into(),
                ..Default::default()
            };

            assert_eq!(expected, data);
        }

        mod fixture_extraction {
            use super::{assert_eq, *};

            #[test]
            fn rename() {
                let data = parse_rstest(
                    r#"long_fixture_name(42, "other") as short, simple as s, no_change()"#,
                );

                let expected = RsTestInfo {
                    data: vec![
                        fixture("short", &["42", r#""other""#])
                            .with_resolve("long_fixture_name")
                            .into(),
                        fixture("s", &[]).with_resolve("simple").into(),
                        fixture("no_change", &[]).into(),
                    ]
                    .into(),
                    ..Default::default()
                };

                assert_eq!(expected, data);
            }

            #[test]
            fn rename_with_attributes() {
                let mut item_fn = r#"
                    fn test_fn(
                        #[from(long_fixture_name)] 
                        #[with(42, "other")] short: u32, 
                        #[from(simple)]
                        s: &str,
                        no_change: i32) {
                    }
                    "#
                .ast();

                let expected = RsTestInfo {
                    data: vec![
                        fixture("short", &["42", r#""other""#])
                            .with_resolve("long_fixture_name")
                            .into(),
                        fixture("s", &[]).with_resolve("simple").into(),
                    ]
                    .into(),
                    ..Default::default()
                };

                let mut data = RsTestInfo::default();

                data.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                assert_eq!(expected, data);
            }

            #[test]
            fn defined_via_with_attributes() {
                let mut item_fn = r#"
                    fn test_fn(#[with(42, "other")] my_fixture: u32, #[with(vec![42])] other: &str) {
                    }
                    "#
                .ast();

                let expected = RsTestInfo {
                    data: vec![
                        fixture("my_fixture", &["42", r#""other""#]).into(),
                        fixture("other", &["vec![42]"]).into(),
                    ]
                    .into(),
                    ..Default::default()
                };

                let mut data = RsTestInfo::default();

                data.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                assert_eq!(expected, data);
            }
        }

        #[test]
        fn empty_fixtures() {
            let data = parse_rstest(r#"::trace::no_trace(some)"#);

            let expected = RsTestInfo {
                attributes: Attributes {
                    attributes: vec![
                        Attribute::attr("trace"),
                        Attribute::tagged("no_trace", vec!["some"]),
                    ],
                }
                .into(),
                ..Default::default()
            };

            assert_eq!(expected, data);
        }

        #[test]
        fn empty_attributes() {
            let data = parse_rstest(r#"my_fixture(42, "other")"#);

            let expected = RsTestInfo {
                data: vec![fixture("my_fixture", &["42", r#""other""#]).into()].into(),
                ..Default::default()
            };

            assert_eq!(expected, data);
        }

        #[test]
        fn extract_notrace_args_atttribute() {
            let mut item_fn = r#"
            fn test_fn(#[notrace] a: u32, #[something_else] b: &str, #[notrace] c: i32) {
            }
            "#
            .ast();

            let mut info = RsTestInfo::default();

            info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                .unwrap();
            info.attributes.add_trace(ident("trace"));

            assert!(!info.attributes.trace_me(&ident("a")));
            assert!(info.attributes.trace_me(&ident("b")));
            assert!(!info.attributes.trace_me(&ident("c")));
            let b_args = item_fn
                .sig
                .inputs
                .into_iter()
                .nth(1)
                .and_then(|id| match id {
                    syn::FnArg::Typed(arg) => Some(arg.attrs),
                    _ => None,
                })
                .unwrap();
            assert_eq!(attrs("#[something_else]"), b_args);
        }

        #[rstest]
        fn extract_future() {
            let mut item_fn = "fn f(#[future] a: u32, b: u32) {}".ast();
            let expected = "fn f(a: u32, b: u32) {}".ast();

            let mut info = RsTestInfo::default();

            info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                .unwrap();

            assert_eq!(item_fn, expected);
            assert!(info.arguments.is_future(&ident("a")));
            assert!(!info.arguments.is_future(&ident("b")));
        }
    }

    mod parametrize_cases {
        use super::{assert_eq, *};
        use std::iter::FromIterator;

        #[test]
        fn one_simple_case_one_arg() {
            let data = parse_rstest(r#"arg, case(42)"#).data;

            let args = data.case_args().collect::<Vec<_>>();
            let cases = data.cases().collect::<Vec<_>>();

            assert_eq!(1, args.len());
            assert_eq!(1, cases.len());
            assert_eq!("arg", &args[0].to_string());
            assert_eq!(to_args!(["42"]), cases[0].args())
        }

        #[test]
        fn happy_path() {
            let info = parse_rstest(
                r#"
                my_fixture(42,"foo"),
                arg1, arg2, arg3,
                case(1,2,3),
                case(11,12,13),
                case(21,22,23)
            "#,
            );

            let data = info.data;
            let fixtures = data.fixtures().cloned().collect::<Vec<_>>();

            assert_eq!(vec![fixture("my_fixture", &["42", r#""foo""#])], fixtures);
            assert_eq!(
                to_strs!(vec!["arg1", "arg2", "arg3"]),
                data.case_args()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            );

            let cases = data.cases().collect::<Vec<_>>();

            assert_eq!(3, cases.len());
            assert_eq!(to_args!(["1", "2", "3"]), cases[0].args());
            assert_eq!(to_args!(["11", "12", "13"]), cases[1].args());
            assert_eq!(to_args!(["21", "22", "23"]), cases[2].args());
        }

        mod defined_via_with_attributes {
            use super::{assert_eq, *};

            #[test]
            fn one_case() {
                let mut item_fn = r#"
                #[case::first(42, "first")]
                fn test_fn(#[case] arg1: u32, #[case] arg2: &str) {
                }
                "#
                .ast();

                let mut info = RsTestInfo::default();

                info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                let case_args = info.data.case_args().cloned().collect::<Vec<_>>();
                let cases = info.data.cases().cloned().collect::<Vec<_>>();

                assert_eq!(to_idents!(["arg1", "arg2"]), case_args);
                assert_eq!(
                    vec![
                        TestCase::from_iter(["42", r#""first""#].iter()).with_description("first"),
                    ],
                    cases
                );
            }

            #[test]
            fn parse_tuple_value() {
                let mut item_fn = r#"
                #[case(42, (24, "first"))]
                fn test_fn(#[case] arg1: u32, #[case] tupled: (u32, &str)) {
                }
                "#
                .ast();

                let mut info = RsTestInfo::default();

                info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                let cases = info.data.cases().cloned().collect::<Vec<_>>();

                assert_eq!(
                    vec![TestCase::from_iter(["42", r#"(24, "first")"#].iter()),],
                    cases
                );
            }

            #[test]
            fn more_cases() {
                let mut item_fn = r#"
                #[case::first(42, "first")]
                #[case(24, "second")]
                #[case::third(0, "third")]
                fn test_fn(#[case] arg1: u32, #[case] arg2: &str) {
                }
                "#
                .ast();

                let mut info = RsTestInfo::default();

                info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                let case_args = info.data.case_args().cloned().collect::<Vec<_>>();
                let cases = info.data.cases().cloned().collect::<Vec<_>>();

                assert_eq!(to_idents!(["arg1", "arg2"]), case_args);
                assert_eq!(
                    vec![
                        TestCase::from_iter(["42", r#""first""#].iter()).with_description("first"),
                        TestCase::from_iter(["24", r#""second""#].iter()),
                        TestCase::from_iter(["0", r#""third""#].iter()).with_description("third"),
                    ],
                    cases
                );
            }

            #[test]
            fn should_collect_attributes() {
                let mut item_fn = r#"
                    #[first]
                    #[first2(42)]
                    #[case(42)]
                    #[second]
                    #[case(24)]
                    #[global]
                    fn test_fn(#[case] arg: u32) {
                    }
                "#
                .ast();

                let mut info = RsTestInfo::default();

                info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                let cases = info.data.cases().cloned().collect::<Vec<_>>();

                assert_eq!(
                    vec![
                        TestCase::from_iter(["42"].iter()).with_attrs(attrs(
                            "
                                #[first]
                                #[first2(42)]
                            "
                        )),
                        TestCase::from_iter(["24"].iter()).with_attrs(attrs(
                            "
                            #[second]
                        "
                        )),
                    ],
                    cases
                );
            }

            #[test]
            fn should_consume_all_used_attributes() {
                let mut item_fn = r#"
                    #[first]
                    #[first2(42)]
                    #[case(42)]
                    #[second]
                    #[case(24)]
                    #[global]
                    fn test_fn(#[case] arg: u32) {
                    }
                "#
                .ast();

                let mut info = RsTestInfo::default();

                info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                assert_eq!(
                    item_fn.attrs,
                    attrs(
                        "
                        #[global]
                        "
                    )
                );
                assert!(!format!("{:?}", item_fn).contains("case"));
            }

            #[test]
            fn should_report_all_errors() {
                let mut item_fn = r#"
                    #[case(#case_error#)]
                    fn test_fn(#[case] arg: u32, #[with(#fixture_error#)] err_fixture: u32) {
                    }
                "#
                .ast();

                let mut info = RsTestInfo::default();

                let errors = info
                    .extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap_err();

                assert_eq!(2, errors.len());
            }
        }

        #[test]
        fn should_accept_comma_at_the_end_of_cases() {
            let data = parse_rstest(
                r#"
                arg,
                case(42),
            "#,
            )
            .data;

            let args = data.case_args().collect::<Vec<_>>();
            let cases = data.cases().collect::<Vec<_>>();

            assert_eq!(1, args.len());
            assert_eq!(1, cases.len());
            assert_eq!("arg", &args[0].to_string());
            assert_eq!(to_args!(["42"]), cases[0].args())
        }

        #[test]
        #[should_panic]
        fn should_not_accept_invalid_separator_from_args_and_cases() {
            parse_rstest(
                r#"
                ret
                case::should_success(Ok(())),
                case::should_fail(Err("Return Error"))
            "#,
            );
        }

        #[test]
        fn case_could_be_arg_name() {
            let data = parse_rstest(
                r#"
                case,
                case(42)
            "#,
            )
            .data;

            assert_eq!("case", &data.case_args().next().unwrap().to_string());

            let cases = data.cases().collect::<Vec<_>>();

            assert_eq!(1, cases.len());
            assert_eq!(to_args!(["42"]), cases[0].args());
        }
    }

    mod matrix_cases {
        use crate::parse::Attribute;

        use super::{assert_eq, *};

        #[test]
        fn happy_path() {
            let info = parse_rstest(
                r#"
                    expected => [12, 34 * 2],
                    input => [format!("aa_{}", 2), "other"],
                "#,
            );

            let value_ranges = info.data.list_values().collect::<Vec<_>>();
            assert_eq!(2, value_ranges.len());
            assert_eq!(to_args!(["12", "34 * 2"]), value_ranges[0].args());
            assert_eq!(
                to_args!([r#"format!("aa_{}", 2)"#, r#""other""#]),
                value_ranges[1].args()
            );
            assert_eq!(info.attributes, Default::default());
        }

        #[test]
        fn should_parse_attributes_too() {
            let info = parse_rstest(
                r#"
                                        a => [12, 24, 42]
                                        ::trace
                                    "#,
            );

            assert_eq!(
                info.attributes,
                Attributes {
                    attributes: vec![Attribute::attr("trace")]
                }
                .into()
            );
        }

        #[test]
        fn should_parse_injected_fixtures_too() {
            let info = parse_rstest(
                r#"
                a => [12, 24, 42],
                fixture_1(42, "foo"),
                fixture_2("bar")
                "#,
            );

            let fixtures = info.data.fixtures().cloned().collect::<Vec<_>>();

            assert_eq!(
                vec![
                    fixture("fixture_1", &["42", r#""foo""#]),
                    fixture("fixture_2", &[r#""bar""#])
                ],
                fixtures
            );
        }

        #[test]
        #[should_panic(expected = "should not be empty")]
        fn should_not_compile_if_empty_expression_slice() {
            parse_rstest(
                r#"
                invalid => []
                "#,
            );
        }

        mod defined_via_with_attributes {
            use super::{assert_eq, *};

            #[test]
            fn one_arg() {
                let mut item_fn = r#"
                fn test_fn(#[values(1, 2, 1+2)] arg1: u32, #[values(format!("a"), "b b".to_owned(), String::new())] arg2: String) {
                }
                "#
                .ast();

                let mut info = RsTestInfo::default();

                info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                    .unwrap();

                let list_values = info.data.list_values().cloned().collect::<Vec<_>>();

                assert_eq!(2, list_values.len());
                assert_eq!(to_args!(["1", "2", "1+2"]), list_values[0].args());
                assert_eq!(
                    to_args!([r#"format!("a")"#, r#""b b".to_owned()"#, "String::new()"]),
                    list_values[1].args()
                );
            }
        }
    }

    mod json {
        use std::collections::HashSet;

        use rstest_test::assert_in;

        use super::{assert_eq, *};

        #[test]
        fn happy_path() {
            let mut item_fn = r#"
            #[json("resources/tests/*.json")]
            fn base(#[field] age: u16, #[data] user: User, #[field("first_name")] name: String) {
                assert!(age==user.age);
            }
            "#
            .ast();

            let mut info = RsTestInfo::default();

            info.extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                .unwrap();

            let files = info.data.files().unwrap();

            assert_eq!(&Folder::fake(), files.hierarchy());
            assert_eq!([ident("user")], files.data());
            assert_eq!(
                HashSet::<&StructField>::from_iter(vec![
                    &StructField::new(ident("age"), None),
                    &StructField::new(ident("name"), Some("first_name".to_string())),
                ]),
                HashSet::from_iter(files.args())
            );
        }

        #[rstest]
        #[case::field_just_once(
            r#"
            #[json("resources/tests/*.json")]
            fn base(#[field] #[field("first_name")] age: u16) {}"#,
            &["field", "more than once"]
        )]
        #[case::field_without_files(
            r#"
            fn base(#[field] age: u16) {}"#,
            &["field", "files test set"]
        )]
        #[case::field_as_name_value(
            r#"
            fn base(#[field = "first_name"] name: String) {}"#,
            &["field", "expected parentheses"]
        )]
        #[case::data_just_once(
            r#"
            #[json("resources/tests/*.json")]
            fn base(#[data] #[data] user: User) {}"#,
            &["data", "more than once"]
        )]
        #[case::data_wrong_syntax(
            r#"
            #[json("resources/tests/*.json")]
            fn base(#[data()] user: User) {}"#,
            &["unexpected token"]
        )]
        #[case::data_wrong_syntax(
            r#"
            #[json("resources/tests/*.json")]
            fn base(#[data = "some"] user: User) {}"#,
            &["unexpected token"]
        )]
        #[case::data_without_files(
            r#"
            fn base(#[data] user: User) {}"#,
            &["data", "files test set"]
        )]
        fn error(#[case] code: &str, #[case] expected: &[&str]) {
            let mut item_fn = code.ast();

            let mut info = RsTestInfo::default();

            let error_code = info
                .extend_with_function_attrs::<DefaultSysEngine>(&mut item_fn)
                .unwrap_err()
                .to_token_stream()
                .display_code();

            for &e in expected {
                assert_in!(error_code, e);
            }
        }
    }

    mod integrated {
        use super::{assert_eq, *};

        #[test]
        fn should_parse_fixture_cases_and_matrix_in_any_order() {
            let data = parse_rstest(
                r#"
                u,
                m => [1, 2],
                case(42, A{}, D{}),
                a,
                case(43, A{}, D{}),
                the_fixture(42),
                mm => ["f", "oo", "BAR"],
                d
            "#,
            )
            .data;

            let fixtures = data.fixtures().cloned().collect::<Vec<_>>();
            assert_eq!(vec![fixture("the_fixture", &["42"])], fixtures);

            assert_eq!(
                to_strs!(vec!["u", "a", "d"]),
                data.case_args()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            );

            let cases = data.cases().collect::<Vec<_>>();
            assert_eq!(2, cases.len());
            assert_eq!(to_args!(["42", "A{}", "D{}"]), cases[0].args());
            assert_eq!(to_args!(["43", "A{}", "D{}"]), cases[1].args());

            let value_ranges = data.list_values().collect::<Vec<_>>();
            assert_eq!(2, value_ranges.len());
            assert_eq!(to_args!(["1", "2"]), value_ranges[0].args());
            assert_eq!(
                to_args!([r#""f""#, r#""oo""#, r#""BAR""#]),
                value_ranges[1].args()
            );
        }
    }
}
