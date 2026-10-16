//! Target definitions, architectures, and ABI specifications for the Vale compiler backend.

/// Target hardware architecture family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Architecture {
    /// 64-bit Armv8.0-A / Armv8.1-A+ (AArch64 / ARM64 / A64)
    AArch64,
    /// 32-bit Armv7-A / Armv8-A 32-bit mode (ARM / Thumb-2 / AArch32)
    Arm32,
    /// 64-bit x86 (x86_64 / AMD64 / Intel 64)
    X86_64,
    /// 64-bit RISC-V (RV64GC / RV64I + M + A + F + D + C + Zk)
    Riscv64,
}

impl Architecture {
    /// Pointer width in bits for this architecture.
    #[inline]
    pub fn pointer_width_bits(self) -> u32 {
        match self {
            Architecture::AArch64 | Architecture::X86_64 | Architecture::Riscv64 => {
                64
            }
            Architecture::Arm32 => 32,
        }
    }

    /// Pointer and word size in bytes.
    #[inline]
    pub fn pointer_width_bytes(self) -> u32 {
        self.pointer_width_bits() / 8
    }

    /// Default hardware stack alignment requirement in bytes.
    #[inline]
    pub fn stack_alignment_bytes(self) -> u32 {
        match self {
            Architecture::AArch64 | Architecture::X86_64 | Architecture::Riscv64 => {
                16
            } // RISC-V standard ABI requires 16-byte stack alignment
            Architecture::Arm32 => 8, // AAPCS32 requires 8-byte alignment at external boundaries
        }
    }
}

/// Supported target operating systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperatingSystem {
    Linux,
    MacOS,
    Windows,
}

/// Binary object format for the target platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectFormat {
    Elf64,
    Elf32,
    MachO,
    Coff,
}

/// Procedure Call Standard (ABI) variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Abi {
    /// Standard AAPCS64 (ARM IHI 0055B) for 64-bit ARM
    Aapcs64,
    /// Standard AAPCS (ARM EABI / ARM IHI 0042F) for 32-bit ARM (hard-float)
    Aapcs32HardFloat,
    /// AAPCS for 32-bit ARM (soft-float / softfp)
    Aapcs32SoftFloat,
    /// Apple ARM64 Calling Convention (x18 reserved, Mach-O relocations)
    AppleDarwin,
    /// Windows ARM64 Calling Convention (x18 reserved TEB)
    WindowsArm64,
    /// Standard System V AMD64 ABI (Linux, macOS, BSDs)
    SystemVAmd64,
    /// Microsoft x64 Calling Convention (Windows)
    WindowsX64,
    /// Standard RISC-V 64-bit ABI (lp64 / lp64d: a0..a7 for args, fa0..fa7 for floats)
    RiscvLp64d,
}

impl Abi {
    /// Number of general-purpose argument registers provided by this ABI.
    #[inline]
    pub fn gpr_argument_register_count(self) -> usize {
        match self {
            Abi::Aapcs64
            | Abi::AppleDarwin
            | Abi::WindowsArm64
            | Abi::RiscvLp64d => 8, // X0..X7 or a0..a7
            Abi::SystemVAmd64 => 6, // RDI, RSI, RDX, RCX, R8, R9
            Abi::WindowsX64 => 4,   // RCX, RDX, R8, R9
            Abi::Aapcs32HardFloat | Abi::Aapcs32SoftFloat => 4, // r0..r3
        }
    }

    /// Number of floating-point argument registers provided by this ABI.
    #[inline]
    pub fn fpr_argument_register_count(self) -> usize {
        match self {
            Abi::Aapcs64
            | Abi::AppleDarwin
            | Abi::WindowsArm64
            | Abi::RiscvLp64d => 8, // V0..V7 or fa0..fa7
            Abi::SystemVAmd64 => 8,      // XMM0..XMM7
            Abi::WindowsX64 => 4,        // XMM0..XMM3
            Abi::Aapcs32HardFloat => 16, // s0..s15 / d0..d7
            Abi::Aapcs32SoftFloat => 0,  // passed in GPRs
        }
    }
}

/// Architecture extensions and feature gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeatureFlags {
    /// Armv8.1-A Large System Extensions (atomic instructions like LDADD, CAS)
    pub has_lse: bool,
    /// ARMv8 Cryptographic Extensions (PMULL with 1Q/1D/2D, AES, SHA)
    pub has_crypto: bool,
    /// Thumb-2 mode for 32-bit ARM (vs standard ARM 32-bit mode)
    pub is_thumb2: bool,
    /// Hardware VFP/NEON floating point (for 32-bit ARM)
    pub has_vfp: bool,
    /// Position Independent Executable (PIE) mode
    pub is_pie: bool,
    /// ARMv8.4-A Data Independent Timing (DIT)
    pub has_dit: bool,
    /// x86 AES-NI instructions (AESENC, AESDEC, etc.)
    pub has_aesni: bool,
    /// x86 Carry-less Multiplication (PCLMULQDQ)
    pub has_pclmulqdq: bool,
    /// x86 SHA Extensions (SHA256RNDS2, etc.)
    pub has_sha_ni: bool,
    /// x86 AVX2 vector extensions (256-bit SIMD)
    pub has_avx2: bool,
    /// Intel Data Operand Independent Timing Mode (DOITM)
    pub has_doitm: bool,
    /// RISC-V Scalar Cryptography Extension (Zk / Zkn / Zks: AES, SHA-256, SHA-512, SM3, SM4)
    pub has_riscv_zk: bool,
    /// RISC-V Bitmanip Extension (Zba / Zbb / Zbs)
    pub has_riscv_zbb: bool,
}

/// Complete target descriptor for code generation and assembly emission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDescriptor {
    pub architecture: Architecture,
    pub os: OperatingSystem,
    pub object_format: ObjectFormat,
    pub abi: Abi,
    pub features: FeatureFlags,
}

impl TargetDescriptor {
    /// Standard Linux AArch64 ELF target (aarch64-unknown-linux-gnu / musl).
    pub fn linux_gnu() -> Self {
        Self::aarch64_linux()
    }

    /// Explicit AArch64 Linux target.
    pub fn aarch64_linux() -> Self {
        Self {
            architecture: Architecture::AArch64,
            os: OperatingSystem::Linux,
            object_format: ObjectFormat::Elf64,
            abi: Abi::Aapcs64,
            features: FeatureFlags {
                has_lse: false,
                has_crypto: false,
                is_thumb2: false,
                has_vfp: true,
                is_pie: true,
                has_dit: false,
                ..Default::default()
            },
        }
    }

    /// Apple macOS AArch64 Mach-O target (aarch64-apple-darwin).
    pub fn macos() -> Self {
        Self {
            architecture: Architecture::AArch64,
            os: OperatingSystem::MacOS,
            object_format: ObjectFormat::MachO,
            abi: Abi::AppleDarwin,
            features: FeatureFlags {
                has_lse: false,
                has_crypto: false,
                is_thumb2: false,
                has_vfp: true,
                is_pie: true,
                has_dit: false,
                ..Default::default()
            },
        }
    }

    /// Builder to enable ARMv8 Cryptographic extensions (PMULL 1Q/1D/2D, AES, SHA).
    pub fn with_crypto(mut self) -> Self {
        self.features.has_crypto = true;
        self
    }

    /// Builder to enable ARMv8.4-A Data Independent Timing (DIT).
    pub fn with_dit(mut self) -> Self {
        self.features.has_dit = true;
        self
    }

    /// Standard Linux ARM32 (ARMv7-A) Hard-Float target (armv7-unknown-linux-gnueabihf).
    pub fn arm32_linux_gnueabihf() -> Self {
        Self {
            architecture: Architecture::Arm32,
            os: OperatingSystem::Linux,
            object_format: ObjectFormat::Elf32,
            abi: Abi::Aapcs32HardFloat,
            features: FeatureFlags {
                has_lse: false,
                has_crypto: false,
                is_thumb2: true,
                has_vfp: true,
                is_pie: true,
                has_dit: false,
                ..Default::default()
            },
        }
    }

    /// Standard Linux x86_64 ELF target (x86_64-unknown-linux-gnu / musl).
    pub fn x86_64_linux() -> Self {
        Self {
            architecture: Architecture::X86_64,
            os: OperatingSystem::Linux,
            object_format: ObjectFormat::Elf64,
            abi: Abi::SystemVAmd64,
            features: FeatureFlags {
                is_pie: true,
                ..Default::default()
            },
        }
    }

    /// Standard Windows x86_64 PE/COFF target (x86_64-pc-windows-msvc / gnu).
    pub fn x86_64_windows() -> Self {
        Self {
            architecture: Architecture::X86_64,
            os: OperatingSystem::Windows,
            object_format: ObjectFormat::Coff,
            abi: Abi::WindowsX64,
            features: FeatureFlags {
                is_pie: true,
                ..Default::default()
            },
        }
    }

    /// Builder to enable x86 AES-NI instructions.
    pub fn with_aesni(mut self) -> Self {
        self.features.has_aesni = true;
        self
    }

    /// Builder to enable x86 PCLMULQDQ instructions.
    pub fn with_pclmulqdq(mut self) -> Self {
        self.features.has_pclmulqdq = true;
        self
    }

    /// Builder to enable x86 SHA Extensions.
    pub fn with_sha_ni(mut self) -> Self {
        self.features.has_sha_ni = true;
        self
    }

    /// Builder to enable Intel DOITM constant-time execution mode.
    pub fn with_doitm(mut self) -> Self {
        self.features.has_doitm = true;
        self
    }

    /// Standard Linux RISC-V 64-bit ELF target (riscv64-unknown-linux-gnu / musl).
    pub fn riscv64_linux() -> Self {
        Self {
            architecture: Architecture::Riscv64,
            os: OperatingSystem::Linux,
            object_format: ObjectFormat::Elf64,
            abi: Abi::RiscvLp64d,
            features: FeatureFlags {
                has_lse: false,
                has_crypto: false,
                is_thumb2: false,
                has_vfp: true,
                is_pie: true,
                has_dit: false,
                has_aesni: false,
                has_pclmulqdq: false,
                has_sha_ni: false,
                has_avx2: false,
                has_doitm: false,
                has_riscv_zk: false,
                has_riscv_zbb: false,
            },
        }
    }

    /// Builder to enable RISC-V Scalar Cryptography (Zk) extension.
    pub fn with_riscv_zk(mut self) -> Self {
        self.features.has_riscv_zk = true;
        self
    }

    /// Builder to enable RISC-V Bitmanip (Zbb) extension.
    pub fn with_riscv_zbb(mut self) -> Self {
        self.features.has_riscv_zbb = true;
        self
    }

    /// Pointer width in bits.
    #[inline]
    pub fn pointer_width_bits(&self) -> u32 {
        self.architecture.pointer_width_bits()
    }

    /// Pointer width in bytes.
    #[inline]
    pub fn pointer_width_bytes(&self) -> u32 {
        self.architecture.pointer_width_bytes()
    }

    /// Stack alignment requirement in bytes.
    #[inline]
    pub fn stack_alignment_bytes(&self) -> u32 {
        self.architecture.stack_alignment_bytes()
    }

    /// Returns the canonical target triple string.
    pub fn triple(&self) -> String {
        let arch_str = match self.architecture {
            Architecture::AArch64 => "aarch64",
            Architecture::Arm32 => "armv7",
            Architecture::X86_64 => "x86_64",
            Architecture::Riscv64 => "riscv64",
        };
        let os_str = match self.os {
            OperatingSystem::Linux => "linux",
            OperatingSystem::MacOS => "macos",
            OperatingSystem::Windows => "windows",
        };
        format!("{}-{}", arch_str, os_str)
    }

    /// Detects the host target descriptor at runtime.
    pub fn host() -> Self {
        #[cfg(target_arch = "aarch64")]
        let arch = Architecture::AArch64;
        #[cfg(target_arch = "arm")]
        let arch = Architecture::Arm32;
        #[cfg(target_arch = "x86_64")]
        let arch = Architecture::X86_64;
        #[cfg(target_arch = "riscv64")]
        let arch = Architecture::Riscv64;
        #[cfg(not(any(
            target_arch = "aarch64",
            target_arch = "arm",
            target_arch = "x86_64",
            target_arch = "riscv64"
        )))]
        let arch = Architecture::AArch64;

        #[cfg(any(target_os = "linux", target_os = "android"))]
        let os = OperatingSystem::Linux;
        #[cfg(target_os = "macos")]
        let os = OperatingSystem::MacOS;
        #[cfg(target_os = "windows")]
        let os = OperatingSystem::Windows;
        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "windows"
        )))]
        let os = OperatingSystem::Linux;

        match (arch, os) {
            (Architecture::AArch64, OperatingSystem::MacOS) => Self::macos(),
            (Architecture::AArch64, _) => Self::aarch64_linux(),
            (Architecture::Arm32, _) => Self::arm32_linux_gnueabihf(),
            (Architecture::X86_64, OperatingSystem::Windows) => {
                Self::x86_64_windows()
            }
            (Architecture::X86_64, _) => Self::x86_64_linux(),
            (Architecture::Riscv64, _) => Self::riscv64_linux(),
        }
    }

    /// Returns whether this target matches the host architecture and OS.
    pub fn is_host(&self) -> bool {
        let host = Self::host();
        self.architecture == host.architecture && self.os == host.os
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aarch64_target_properties() {
        let t = TargetDescriptor::aarch64_linux();
        assert_eq!(t.architecture, Architecture::AArch64);
        assert_eq!(t.pointer_width_bits(), 64);
        assert_eq!(t.pointer_width_bytes(), 8);
        assert_eq!(t.stack_alignment_bytes(), 16);
        assert_eq!(t.abi.gpr_argument_register_count(), 8);
    }

    #[test]
    fn test_arm32_target_properties() {
        let t = TargetDescriptor::arm32_linux_gnueabihf();
        assert_eq!(t.architecture, Architecture::Arm32);
        assert_eq!(t.pointer_width_bits(), 32);
        assert_eq!(t.pointer_width_bytes(), 4);
        assert_eq!(t.stack_alignment_bytes(), 8);
        assert_eq!(t.abi.gpr_argument_register_count(), 4);
    }

    #[test]
    fn test_x86_64_target_properties() {
        let t = TargetDescriptor::x86_64_linux().with_aesni().with_sha_ni();
        assert_eq!(t.architecture, Architecture::X86_64);
        assert_eq!(t.pointer_width_bits(), 64);
        assert_eq!(t.pointer_width_bytes(), 8);
        assert_eq!(t.stack_alignment_bytes(), 16);
        assert_eq!(t.abi.gpr_argument_register_count(), 6);
        assert_eq!(t.abi.fpr_argument_register_count(), 8);
        assert!(t.features.has_aesni);
        assert!(t.features.has_sha_ni);
    }

    #[test]
    fn test_riscv64_target_properties() {
        let t = TargetDescriptor::riscv64_linux()
            .with_riscv_zk()
            .with_riscv_zbb();
        assert_eq!(t.architecture, Architecture::Riscv64);
        assert_eq!(t.pointer_width_bits(), 64);
        assert_eq!(t.pointer_width_bytes(), 8);
        assert_eq!(t.stack_alignment_bytes(), 16);
        assert_eq!(t.abi.gpr_argument_register_count(), 8);
        assert_eq!(t.abi.fpr_argument_register_count(), 8);
        assert!(t.features.has_riscv_zk);
        assert!(t.features.has_riscv_zbb);
    }
}
