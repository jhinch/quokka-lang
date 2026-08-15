use crate::ast::{Identifier, SpannedIdentifier};
use crate::attr::FieldDeclAttributes;
use indexmap::map::Entry;
use indexmap::{IndexMap, IndexSet};
use miette::SourceSpan;
use std::fmt;
use std::hash::Hash;
use std::num::NonZeroU16;

#[derive(Default)]
pub struct TyRegistry {
    struct_types: IndexMap<Identifier, StructTyData>,
    array_types: IndexSet<ArrayTyData>,
    fn_ptr_types: IndexSet<FnPtrTyData>,
}

impl TyRegistry {
    #[must_use]
    pub fn resolve(&self, name: &Identifier) -> Option<Ty> {
        if let Some((index, _, _)) = self.struct_types.get_full(name) {
            return Some(Ty::struct_ty(index));
        }
        self.resolve_builtin(name.value.as_str())
    }

    fn resolve_builtin(&self, name: &str) -> Option<Ty> {
        Some(match name {
            "never" => Ty::NEVER,
            "unit" => Ty::UNIT,
            "u1" => Ty::U1,
            "u2" => Ty::U2,
            "u4" => Ty::U4,
            "u8" => Ty::U8,
            "u16" => Ty::U16,
            "i8" => Ty::I8,
            "i16" => Ty::I16,
            _ => return None,
        })
    }

    pub fn register_struct(&mut self, name: SpannedIdentifier) -> Result<Ty, SourceSpan> {
        match self.struct_types.entry(name.data.clone()) {
            Entry::Occupied(entry) => Err(entry.get().name.span),
            Entry::Vacant(entry) => {
                let index = entry
                    .insert_entry(StructTyData {
                        name,
                        fields: IndexMap::new(),
                        bit_size: u32::MAX,
                    })
                    .index();
                Ok(Ty::struct_ty(index))
            }
        }
    }

    pub fn register_array(&mut self, item_ty: Ty, count: NonZeroU16) -> Ty {
        let (index, _) = self.array_types.insert_full(ArrayTyData { item_ty, count });
        Ty::array_ty(index)
    }

    pub fn register_fn_ptr(&mut self, return_ty: Ty) -> Ty {
        let (index, _) = self.fn_ptr_types.insert_full(FnPtrTyData { return_ty });
        Ty::fn_ptr_ty(index)
    }

    pub fn complete_struct_registration(
        &mut self,
        ty: Ty,
        fields: IndexMap<Identifier, StructFieldTyData>,
        bit_size: u32,
    ) {
        assert!(ty.is_struct(), "{ty:?} not a struct");
        let data = &mut self.struct_types[ty.index()];
        assert_eq!(data.bit_size, u32::MAX, "{ty:?} is already registered");
        assert_ne!(bit_size, u32::MAX, "bit_size must be smaller than u32::MAX");
        data.fields = fields;
        data.bit_size = bit_size;
    }

    #[must_use]
    pub fn struct_ty_data(&self, ty: Ty) -> &StructTyData {
        assert!(ty.is_struct(), "{ty:?} not a struct");
        let data = &self.struct_types[ty.index()];
        assert_ne!(data.bit_size, u32::MAX, "{ty:?} not fully registered");
        data
    }

    #[must_use]
    pub fn array_ty_data(&self, ty: Ty) -> &ArrayTyData {
        assert!(ty.is_array(), "{ty:?} not an array");
        &self.array_types[ty.index()]
    }

    #[must_use]
    pub fn fn_ptr_ty_data(&self, ty: Ty) -> &FnPtrTyData {
        assert!(ty.is_fn_ptr(), "{ty:?} not an fn pointer");
        &self.fn_ptr_types[ty.index()]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ty(u32);

impl Ty {
    pub const UNRESOLVED: Ty = Ty(Ty::BUILTIN_TAG /* | 0 */);
    pub const ERROR: Ty = Ty(Ty::BUILTIN_TAG | 1);
    pub const NEVER: Ty = Ty(Ty::BUILTIN_TAG | 2);
    pub const UNIT: Ty = Ty(Ty::BUILTIN_TAG | 3);
    pub const U1: Ty = Ty(Ty::BUILTIN_TAG | 4);
    pub const U2: Ty = Ty(Ty::BUILTIN_TAG | 5);
    pub const U4: Ty = Ty(Ty::BUILTIN_TAG | 6);
    pub const U8: Ty = Ty(Ty::BUILTIN_TAG | 7);
    pub const U16: Ty = Ty(Ty::BUILTIN_TAG | 8);
    pub const I8: Ty = Ty(Ty::BUILTIN_TAG | 9);
    pub const I16: Ty = Ty(Ty::BUILTIN_TAG | 10);

    const BUILTIN_TAG: u32 = 0b0000 << 28;
    const ARRAY_TAG: u32 = 0b0001 << 28;
    const STRUCT_TAG: u32 = 0b0010 << 28;
    const FN_PTR_TAG: u32 = 0b0011 << 28;
    const TAG_MASK: u32 = 0b1111 << 28;
    const INDEX_MASK: u32 = !Ty::TAG_MASK;

    #[must_use]
    pub fn struct_ty(index: usize) -> Self {
        let index = u32::try_from(index)
            .ok()
            .filter(|index| *index <= Ty::INDEX_MASK)
            .unwrap_or_else(|| panic!("Too many struct types. {} > {}", index, Ty::INDEX_MASK));
        Ty(Ty::STRUCT_TAG | index)
    }

    #[must_use]
    pub fn array_ty(index: usize) -> Self {
        let index = u32::try_from(index)
            .ok()
            .filter(|index| *index <= Ty::INDEX_MASK)
            .unwrap_or_else(|| panic!("Too many array types. {} > {}", index, Ty::INDEX_MASK));
        Ty(Ty::ARRAY_TAG | index)
    }

    #[must_use]
    pub fn fn_ptr_ty(index: usize) -> Self {
        let index = u32::try_from(index)
            .ok()
            .filter(|index| *index <= Ty::INDEX_MASK)
            .unwrap_or_else(|| panic!("Too many fn pointer types. {} > {}", index, Ty::INDEX_MASK));
        Ty(Ty::FN_PTR_TAG | index)
    }

    #[must_use]
    pub const fn is_builtin(self) -> bool {
        self.0 & Ty::TAG_MASK == Ty::BUILTIN_TAG
    }

    #[must_use]
    pub const fn is_int(self) -> bool {
        self.0 >= Ty::U1.0 && self.0 <= Ty::I16.0
    }

    #[must_use]
    pub const fn is_unsigned_int(self) -> bool {
        self.0 >= Ty::U1.0 && self.0 <= Ty::U16.0
    }

    #[must_use]
    pub const fn is_signed_int(self) -> bool {
        self.0 >= Ty::I8.0 && self.0 <= Ty::I16.0
    }

    #[must_use]
    pub const fn is_struct(self) -> bool {
        self.0 & Ty::TAG_MASK == Ty::STRUCT_TAG
    }

    #[must_use]
    pub const fn is_array(self) -> bool {
        self.0 & Ty::TAG_MASK == Ty::ARRAY_TAG
    }

    #[must_use]
    pub const fn is_fn_ptr(self) -> bool {
        self.0 & Ty::TAG_MASK == Ty::FN_PTR_TAG
    }

    #[must_use]
    pub fn display(self, registry: &TyRegistry) -> TyDisplay<'_> {
        TyDisplay {
            data: TyDisplayData::Deferred(registry, self),
        }
    }

    #[must_use]
    pub fn bit_size(self, registry: &TyRegistry) -> u32 {
        match self {
            Ty::U1 => 1,
            Ty::U2 => 2,
            Ty::U4 => 4,
            Ty::U8 | Ty::I8 => 8,
            Ty::U16 | Ty::I16 => 16,
            ty if ty.is_array() => {
                let data = registry.array_ty_data(ty);
                data.item_ty.bit_size(registry).next_multiple_of(8) * (data.count.get() as u32)
            }
            ty if ty.is_struct() => registry.struct_ty_data(ty).bit_size,
            ty if ty.is_fn_ptr() => 16,
            _ => 0,
        }
    }

    const fn index(self) -> usize {
        (self.0 & Ty::INDEX_MASK) as usize
    }
}

impl Default for Ty {
    fn default() -> Self {
        Ty::UNRESOLVED
    }
}

impl fmt::Debug for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Ty::UNRESOLVED => f.write_str("<unresolved>"),
            Ty::ERROR => f.write_str("<error>"),
            Ty::NEVER => f.write_str("never"),
            Ty::UNIT => f.write_str("unit"),
            Ty::U1 => f.write_str("u1"),
            Ty::U2 => f.write_str("u2"),
            Ty::U4 => f.write_str("u4"),
            Ty::U8 => f.write_str("u8"),
            Ty::U16 => f.write_str("u16"),
            Ty::I8 => f.write_str("i8"),
            Ty::I16 => f.write_str("i16"),
            ty if ty.is_array() => write!(f, "array({})", ty.index()),
            ty if ty.is_struct() => write!(f, "struct({})", ty.index()),
            ty if ty.is_fn_ptr() => write!(f, "fn_ptr({})", ty.index()),
            ty => write!(f, "Ty({:#034b}", ty.0),
        }
    }
}

#[derive(Debug)]
pub struct StructTyData {
    pub name: SpannedIdentifier,
    pub fields: IndexMap<Identifier, StructFieldTyData>,
    pub bit_size: u32,
}

#[derive(Debug)]
pub struct StructFieldTyData {
    pub name: SpannedIdentifier,
    pub ty: Ty,
    pub ty_span: SourceSpan,
    pub bit_offset: u32,
    pub attrs: FieldDeclAttributes,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct ArrayTyData {
    pub item_ty: Ty,
    pub count: NonZeroU16,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct FnPtrTyData {
    pub return_ty: Ty,
}

pub struct TyDisplay<'a> {
    data: TyDisplayData<'a>,
}

enum TyDisplayData<'a> {
    Deferred(&'a TyRegistry, Ty),
    Rendered(String),
}

impl TyDisplay<'_> {
    #[must_use]
    pub fn into_detached(self) -> TyDisplay<'static> {
        let rendered = match self.data {
            TyDisplayData::Deferred(registry, ty) => format!("{}", ty.display(registry)),
            TyDisplayData::Rendered(rendered) => rendered,
        };
        TyDisplay {
            data: TyDisplayData::Rendered(rendered),
        }
    }
}

impl fmt::Display for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            TyDisplayData::Deferred(registry, ty) => match *ty {
                ty if ty.is_array() => {
                    let data = registry.array_ty_data(ty);
                    write!(f, "[{}; {}]", data.item_ty.display(registry), data.count)
                }
                ty if ty.is_struct() => {
                    let data = registry.struct_ty_data(ty);
                    write!(f, "{}", data.name.data)
                }
                ty if ty.is_fn_ptr() => {
                    let data = registry.fn_ptr_ty_data(ty);
                    write!(f, "fn() -> {}", data.return_ty.display(registry))
                }
                ty => fmt::Debug::fmt(&ty, f),
            },
            TyDisplayData::Rendered(rendered) => f.write_str(rendered),
        }
    }
}

impl fmt::Debug for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            TyDisplayData::Deferred(_, ty) => write!(f, "{ty:?}"),
            TyDisplayData::Rendered(rendered) => write!(f, "{rendered}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::SourceOffset;

    fn ident(name: &str) -> SpannedIdentifier {
        SpannedIdentifier::new(name, SourceSpan::new(SourceOffset::from(0), 0))
    }

    #[test]
    fn builtin_tys_are_builtin_and_not_struct_array_or_fn_ptr() {
        for ty in [
            Ty::UNRESOLVED,
            Ty::ERROR,
            Ty::NEVER,
            Ty::UNIT,
            Ty::U1,
            Ty::U2,
            Ty::U4,
            Ty::U8,
            Ty::U16,
            Ty::I8,
            Ty::I16,
        ] {
            assert!(ty.is_builtin(), "{ty:?} should be builtin");
            assert!(!ty.is_struct(), "{ty:?} should not be struct");
            assert!(!ty.is_array(), "{ty:?} should not be array");
            assert!(!ty.is_fn_ptr(), "{ty:?} should not be fn_ptr");
        }
    }

    #[test]
    fn is_int_covers_only_u1_through_i16() {
        assert!(!Ty::UNRESOLVED.is_int());
        assert!(!Ty::ERROR.is_int());
        assert!(!Ty::NEVER.is_int());
        assert!(!Ty::UNIT.is_int());
        for ty in [Ty::U1, Ty::U2, Ty::U4, Ty::U8, Ty::U16, Ty::I8, Ty::I16] {
            assert!(ty.is_int(), "{ty:?} should be int");
        }
    }

    #[test]
    fn is_unsigned_int_excludes_signed() {
        for ty in [Ty::U1, Ty::U2, Ty::U4, Ty::U8, Ty::U16] {
            assert!(ty.is_unsigned_int(), "{ty:?} should be unsigned");
            assert!(!ty.is_signed_int(), "{ty:?} should not be signed");
        }
        for ty in [Ty::I8, Ty::I16] {
            assert!(!ty.is_unsigned_int(), "{ty:?} should not be unsigned");
            assert!(ty.is_signed_int(), "{ty:?} should be signed");
        }
    }

    #[test]
    fn struct_array_fn_ptr_tys_are_tagged_correctly() {
        let struct_ty = Ty::struct_ty(3);
        assert!(struct_ty.is_struct());
        assert!(!struct_ty.is_array());
        assert!(!struct_ty.is_fn_ptr());
        assert!(!struct_ty.is_builtin());
        assert!(!struct_ty.is_int());

        let array_ty = Ty::array_ty(5);
        assert!(array_ty.is_array());
        assert!(!array_ty.is_struct());
        assert!(!array_ty.is_fn_ptr());

        let fn_ptr_ty = Ty::fn_ptr_ty(7);
        assert!(fn_ptr_ty.is_fn_ptr());
        assert!(!fn_ptr_ty.is_struct());
        assert!(!fn_ptr_ty.is_array());
    }

    #[test]
    fn different_kinds_with_same_index_are_not_equal() {
        assert_ne!(Ty::struct_ty(0), Ty::array_ty(0));
        assert_ne!(Ty::struct_ty(0), Ty::fn_ptr_ty(0));
        assert_ne!(Ty::array_ty(0), Ty::fn_ptr_ty(0));
    }

    #[test]
    fn same_kind_and_index_are_equal() {
        assert_eq!(Ty::struct_ty(2), Ty::struct_ty(2));
        assert_eq!(Ty::array_ty(2), Ty::array_ty(2));
        assert_eq!(Ty::fn_ptr_ty(2), Ty::fn_ptr_ty(2));
    }

    #[test]
    #[should_panic(expected = "Too many struct types")]
    fn struct_ty_panics_when_index_too_large() {
        let _ = Ty::struct_ty(Ty::INDEX_MASK as usize + 1);
    }

    #[test]
    #[should_panic(expected = "Too many array types")]
    fn array_ty_panics_when_index_too_large() {
        let _ = Ty::array_ty(Ty::INDEX_MASK as usize + 1);
    }

    #[test]
    #[should_panic(expected = "Too many fn pointer types")]
    fn fn_ptr_ty_panics_when_index_too_large() {
        let _ = Ty::fn_ptr_ty(Ty::INDEX_MASK as usize + 1);
    }

    #[test]
    fn default_ty_is_unresolved() {
        assert_eq!(Ty::default(), Ty::UNRESOLVED);
    }

    #[test]
    fn debug_format_for_builtins() {
        assert_eq!(format!("{:?}", Ty::UNRESOLVED), "<unresolved>");
        assert_eq!(format!("{:?}", Ty::ERROR), "<error>");
        assert_eq!(format!("{:?}", Ty::NEVER), "never");
        assert_eq!(format!("{:?}", Ty::UNIT), "unit");
        assert_eq!(format!("{:?}", Ty::U1), "u1");
        assert_eq!(format!("{:?}", Ty::I16), "i16");
    }

    #[test]
    fn debug_format_for_struct_array_fn_ptr() {
        assert_eq!(format!("{:?}", Ty::struct_ty(1)), "struct(1)");
        assert_eq!(format!("{:?}", Ty::array_ty(2)), "array(2)");
        assert_eq!(format!("{:?}", Ty::fn_ptr_ty(3)), "fn_ptr(3)");
    }

    #[test]
    fn bit_size_of_builtins() {
        let registry = TyRegistry::default();
        assert_eq!(Ty::U1.bit_size(&registry), 1);
        assert_eq!(Ty::U2.bit_size(&registry), 2);
        assert_eq!(Ty::U4.bit_size(&registry), 4);
        assert_eq!(Ty::U8.bit_size(&registry), 8);
        assert_eq!(Ty::I8.bit_size(&registry), 8);
        assert_eq!(Ty::U16.bit_size(&registry), 16);
        assert_eq!(Ty::I16.bit_size(&registry), 16);
        assert_eq!(Ty::UNIT.bit_size(&registry), 0);
        assert_eq!(Ty::NEVER.bit_size(&registry), 0);
    }

    #[test]
    fn bit_size_of_array_rounds_item_up_to_byte_and_multiplies_by_count() {
        let mut registry = TyRegistry::default();
        // u1 items round up to 8 bits each, times 3 items = 24
        let array_ty = registry.register_array(Ty::U1, NonZeroU16::new(3).unwrap());
        assert_eq!(array_ty.bit_size(&registry), 24);

        // u16 items are already byte-aligned: 16 * 4 = 64
        let array_ty2 = registry.register_array(Ty::U16, NonZeroU16::new(4).unwrap());
        assert_eq!(array_ty2.bit_size(&registry), 64);
    }

    #[test]
    fn bit_size_of_fn_ptr_is_16() {
        let mut registry = TyRegistry::default();
        let fn_ptr = registry.register_fn_ptr(Ty::UNIT);
        assert_eq!(fn_ptr.bit_size(&registry), 16);
    }

    #[test]
    fn bit_size_of_struct_uses_registered_bit_size() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_struct(ident("Foo")).unwrap();
        registry.complete_struct_registration(ty, IndexMap::new(), 40);
        assert_eq!(ty.bit_size(&registry), 40);
    }

    #[test]
    fn resolve_builtin_names() {
        let registry = TyRegistry::default();
        assert_eq!(registry.resolve(&Identifier::from("u8")), Some(Ty::U8));
        assert_eq!(
            registry.resolve(&Identifier::from("never")),
            Some(Ty::NEVER)
        );
        assert_eq!(registry.resolve(&Identifier::from("unit")), Some(Ty::UNIT));
        assert_eq!(registry.resolve(&Identifier::from("i16")), Some(Ty::I16));
    }

    #[test]
    fn resolve_unknown_name_returns_none() {
        let registry = TyRegistry::default();
        assert_eq!(registry.resolve(&Identifier::from("bogus")), None);
    }

    #[test]
    fn resolve_prefers_registered_struct_over_builtin_name_clash() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_struct(ident("u8")).unwrap();
        assert_eq!(registry.resolve(&Identifier::from("u8")), Some(ty));
        assert_ne!(ty, Ty::U8);
    }

    #[test]
    fn register_struct_returns_distinct_tys_for_distinct_names() {
        let mut registry = TyRegistry::default();
        let a = registry.register_struct(ident("A")).unwrap();
        let b = registry.register_struct(ident("B")).unwrap();
        assert_ne!(a, b);
        assert!(a.is_struct());
        assert!(b.is_struct());
    }

    #[test]
    fn register_struct_rejects_duplicate_name_with_span_of_first() {
        let mut registry = TyRegistry::default();
        let first_span = SourceSpan::new(SourceOffset::from(10), 3);
        registry
            .register_struct(SpannedIdentifier::new("Foo", first_span))
            .unwrap();
        let err = registry
            .register_struct(SpannedIdentifier::new(
                "Foo",
                SourceSpan::new(SourceOffset::from(50), 3),
            ))
            .unwrap_err();
        assert_eq!(err.offset(), first_span.offset());
    }

    #[test]
    fn register_array_deduplicates_identical_arrays() {
        let mut registry = TyRegistry::default();
        let a = registry.register_array(Ty::U8, NonZeroU16::new(4).unwrap());
        let b = registry.register_array(Ty::U8, NonZeroU16::new(4).unwrap());
        assert_eq!(a, b);
    }

    #[test]
    fn register_array_distinguishes_by_item_ty_and_count() {
        let mut registry = TyRegistry::default();
        let a = registry.register_array(Ty::U8, NonZeroU16::new(4).unwrap());
        let b = registry.register_array(Ty::U8, NonZeroU16::new(5).unwrap());
        let c = registry.register_array(Ty::U16, NonZeroU16::new(4).unwrap());
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn register_fn_ptr_deduplicates_identical_signatures() {
        let mut registry = TyRegistry::default();
        let a = registry.register_fn_ptr(Ty::U8);
        let b = registry.register_fn_ptr(Ty::U8);
        let c = registry.register_fn_ptr(Ty::U16);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn complete_struct_registration_stores_fields_and_bit_size() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_struct(ident("Foo")).unwrap();
        let mut fields = IndexMap::new();
        fields.insert(
            Identifier::from("x"),
            StructFieldTyData {
                name: ident("x"),
                ty: Ty::U8,
                ty_span: SourceSpan::new(SourceOffset::from(0), 0),
                bit_offset: 0,
                attrs: FieldDeclAttributes::default(),
            },
        );
        registry.complete_struct_registration(ty, fields, 8);
        let data = registry.struct_ty_data(ty);
        assert_eq!(data.bit_size, 8);
        assert!(data.fields.contains_key(&Identifier::from("x")));
    }

    #[test]
    #[should_panic(expected = "not a struct")]
    fn complete_struct_registration_panics_on_non_struct_ty() {
        let mut registry = TyRegistry::default();
        registry.complete_struct_registration(Ty::U8, IndexMap::new(), 8);
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn complete_struct_registration_panics_when_already_completed() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_struct(ident("Foo")).unwrap();
        registry.complete_struct_registration(ty, IndexMap::new(), 8);
        registry.complete_struct_registration(ty, IndexMap::new(), 8);
    }

    #[test]
    #[should_panic(expected = "not fully registered")]
    fn struct_ty_data_panics_when_not_completed() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_struct(ident("Foo")).unwrap();
        let _ = registry.struct_ty_data(ty);
    }

    #[test]
    fn display_of_builtin_ty_matches_debug() {
        let registry = TyRegistry::default();
        assert_eq!(format!("{}", Ty::U8.display(&registry)), "u8");
    }

    #[test]
    fn display_of_struct_uses_name() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_struct(ident("Point")).unwrap();
        registry.complete_struct_registration(ty, IndexMap::new(), 16);
        assert_eq!(format!("{}", ty.display(&registry)), "Point");
    }

    #[test]
    fn display_of_array_shows_item_and_count() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_array(Ty::U8, NonZeroU16::new(3).unwrap());
        assert_eq!(format!("{}", ty.display(&registry)), "[u8; 3]");
    }

    #[test]
    fn display_of_fn_ptr_shows_return_ty() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_fn_ptr(Ty::U16);
        assert_eq!(format!("{}", ty.display(&registry)), "fn() -> u16");
    }

    #[test]
    fn into_detached_preserves_rendering_without_registry() {
        let mut registry = TyRegistry::default();
        let ty = registry.register_array(Ty::U8, NonZeroU16::new(2).unwrap());
        let detached = ty.display(&registry).into_detached();
        assert_eq!(format!("{detached}"), "[u8; 2]");
        // detaching again should reuse the already-rendered string
        let detached_again = detached.into_detached();
        assert_eq!(format!("{detached_again}"), "[u8; 2]");
    }
}
