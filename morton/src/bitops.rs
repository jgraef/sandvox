#![allow(unused_macros)]

pub trait BitDeposit {
    fn deposit(self, mask: Self) -> Self;
}

pub trait BitExtract {
    fn extract(self, mask: Self) -> Self;
}

macro_rules! impl_with_instrinsics {
    ($ty:ty, $arch:ident, $pdep:ident, $pext:ident) => {
        impl BitDeposit for $ty {
            #[inline(always)]
            fn deposit(self, mask: $ty) -> $ty {
                unsafe { core::arch::$arch::$pdep(self, mask) }
            }
        }

        impl BitExtract for $ty {
            #[inline(always)]
            fn extract(self, mask: $ty) -> $ty {
                unsafe { core::arch::$arch::$pext(self, mask) }
            }
        }
    };
}

macro_rules! impl_with_cast {
    ($ty:ty as $proxy:ty) => {
        impl BitDeposit for $ty {
            #[inline(always)]
            fn deposit(self, mask: $ty) -> $ty {
                <$proxy as BitDeposit>::deposit(self as $proxy, mask as $proxy) as $ty
            }
        }

        impl BitExtract for $ty {
            #[inline(always)]
            fn extract(self, mask: $ty) -> $ty {
                <$proxy as BitExtract>::extract(self as $proxy, mask as $proxy) as $ty
            }
        }
    };
}

macro_rules! impl_with_fallback {
    ($ty:ty) => {
        impl BitDeposit for $ty {
            #[inline]
            fn deposit(self, mask: $ty) -> $ty {
                <$ty>::deposit_bits(self, mask)
            }
        }

        impl BitExtract for $ty {
            #[inline]
            fn extract(self, mask: $ty) -> $ty {
                <$ty>::extract_bits(self, mask)
            }
        }
    };
}

#[cfg(target_arch = "x86_64")]
const _: () = {
    impl_with_instrinsics!(u64, x86_64, _pdep_u64, _pext_u64);
    impl_with_instrinsics!(u32, x86_64, _pdep_u32, _pext_u32);
    impl_with_cast!(u16 as u32);
};

#[cfg(not(target_arch = "x86_64"))]
const _: () = {
    impl_with_fallback!(u64);
    impl_with_fallback!(u32);
    impl_with_fallback!(u16);
};
