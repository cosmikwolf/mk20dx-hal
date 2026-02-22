pub type Hertz = fugit::HertzU32;
pub type KiloHertz = fugit::KilohertzU32;
pub type MegaHertz = fugit::MegahertzU32;

#[allow(non_snake_case)]
pub trait U32Ext {
    fn Hz(self) -> Hertz;
    fn kHz(self) -> KiloHertz;
    fn MHz(self) -> MegaHertz;
}

#[allow(non_snake_case)]
impl U32Ext for u32 {
    fn Hz(self) -> Hertz {
        Hertz::from_raw(self)
    }

    fn kHz(self) -> KiloHertz {
        KiloHertz::from_raw(self)
    }

    fn MHz(self) -> MegaHertz {
        MegaHertz::from_raw(self)
    }
}
