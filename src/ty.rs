use crate::ast::{Identifier, SpannedIdentifier};
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
        self.resolve_builtin(&name.value)
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

    pub fn register_struct(
        &mut self,
        name: SpannedIdentifier,
        field_count: usize,
    ) -> Result<Ty, SourceSpan> {
        match self.struct_types.entry(name.data.clone()) {
            Entry::Occupied(entry) => Err(entry.get().name.span),
            Entry::Vacant(entry) => {
                let index = entry
                    .insert_entry(StructTyData {
                        name,
                        fields: IndexMap::with_capacity(field_count),
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

    pub fn register_struct_fields(
        &mut self,
        name: &SpannedIdentifier,
        fields: IndexMap<Identifier, Ty>,
    ) {
        self.struct_types[&name.data].fields = fields;
    }

    #[must_use]
    pub fn struct_ty_data(&self, ty: Ty) -> &StructTyData {
        assert!(ty.is_struct(), "{ty:?} not a struct");
        &self.struct_types[ty.index()]
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
        if index > Ty::INDEX_MASK as usize {
            panic!("Too many struct types. {} > {}", index, Ty::INDEX_MASK);
        }
        Ty(Ty::STRUCT_TAG | index as u32)
    }

    #[must_use]
    pub fn array_ty(index: usize) -> Self {
        if index > Ty::INDEX_MASK as usize {
            panic!("Too many array types. {} > {}", index, Ty::INDEX_MASK);
        }
        Ty(Ty::ARRAY_TAG | index as u32)
    }

    #[must_use]
    pub fn fn_ptr_ty(index: usize) -> Self {
        if index > Ty::INDEX_MASK as usize {
            panic!("Too many fn pointer types. {} > {}", index, Ty::INDEX_MASK);
        }
        Ty(Ty::FN_PTR_TAG | index as u32)
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
            registry_or_rendered: RegistryOrRendered::Registry(registry),
            ty: self,
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

pub struct StructTyData {
    pub name: SpannedIdentifier,
    pub fields: IndexMap<Identifier, Ty>,
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
    registry_or_rendered: RegistryOrRendered<'a>,
    ty: Ty,
}

impl TyDisplay<'_> {
    #[must_use]
    pub fn into_detached(self) -> TyDisplay<'static> {
        let rendered = match self.registry_or_rendered {
            RegistryOrRendered::Registry(registry) => format!("{}", self.ty.display(registry)),
            RegistryOrRendered::Rendered(rendered) => rendered,
        };
        TyDisplay {
            registry_or_rendered: RegistryOrRendered::Rendered(rendered),
            ty: self.ty,
        }
    }
}

impl fmt::Display for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.registry_or_rendered {
            RegistryOrRendered::Registry(registry) => match self.ty {
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
            RegistryOrRendered::Rendered(rendered) => f.write_str(rendered),
        }
    }
}

impl fmt::Debug for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.ty, f)
    }
}

enum RegistryOrRendered<'a> {
    Registry(&'a TyRegistry),
    Rendered(String),
}
