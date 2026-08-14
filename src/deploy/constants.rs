pub const CLEAN_SUBSTITUTERS: &str = "https://cache.nixos.org https://nix-community.cachix.org https://r3dlust.cachix.org https://install.determinate.systems";
pub const CLEAN_TRUSTED_PUBLIC_KEYS: &str = "cache.flakehub.com-3:hJuILl5sVK4iKm86JzgdXW12Y2Hwd5G07qKtHTOcDCM= cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs= r3dlust.cachix.org-1:/R3S8pW/nr7kOBJKcGPsZ0zCepvldTUEgbrqa4O3cW0=";
pub const SYSTEM: &str = if cfg!(target_os = "macos") {
    "darwin"
} else {
    "os"
};
