"""Call functions of the original executable under the Unicorn CPU emulator.

Used to produce verified test vectors and data tables (net crypto, message formats) without
running the game. Needs `pip install unicorn pefile` (installed in P:\\a3-rust\\.work\\venv).

    from emu import Emu
    e = Emu("P:/a3-rust/oirignal/arma3_x64.exe")
    buf = e.alloc(16, bytes(16))
    e.call(0x7b4c40, 0, buf, 16, out)      # RVA, then integer/pointer args (Windows x64 ABI)
    e.read(out, 16)

`runtime=True` (default) adds a minimal fake runtime so that more code runs:
- every import resolves to a stub returning 0 (log with `e.import_calls`);
- the engine's global allocator (object at RVA 0x20870d8, used through PTR 0x20870c8) is
  replaced by a bump allocator (alloc/realloc/free hooks);
- GS points to a fake TEB/TLS block so thread-safe statics (`_Init_thread_header`) work.
Functions that need real OS services, files or threads will still fail; the error names the
faulting address.
"""

from __future__ import annotations

import struct

import pefile
from unicorn import UC_ARCH_X86, UC_HOOK_CODE, UC_HOOK_MEM_INVALID, UC_MODE_64, Uc, UcError
from unicorn.x86_const import (UC_X86_REG_GS_BASE, UC_X86_REG_R8, UC_X86_REG_R9, UC_X86_REG_RAX,
                               UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_RIP, UC_X86_REG_RSP)

STACK_TOP = 0x7FFF_0000_0000
STACK_SIZE = 0x200000
HEAP_BASE = 0x6000_0000_0000
HEAP_SIZE = 0x8000000
STUB_BASE = 0x5000_0000_0000
RET_SENTINEL = STUB_BASE
TEB_BASE = 0x5100_0000_0000

ALLOCATOR_OBJ_RVA = 0x20870D8   # engine memory manager instance (vtable ptr written by us)
N_ALLOC_SLOTS = 32


class Emu:
    def __init__(self, exe: str, runtime: bool = True):
        self.pe = pe = pefile.PE(exe, fast_load=True)
        self.base = pe.OPTIONAL_HEADER.ImageBase
        image = pe.get_memory_mapped_image()
        size = (len(image) + 0xFFF) & ~0xFFF
        self.uc = Uc(UC_ARCH_X86, UC_MODE_64)
        self.uc.mem_map(self.base, size)
        self.uc.mem_write(self.base, image)
        self.uc.mem_map(STACK_TOP - STACK_SIZE, STACK_SIZE)
        self.uc.mem_map(HEAP_BASE, HEAP_SIZE)
        self.uc.mem_map(STUB_BASE, 0x10000)
        self.uc.mem_write(STUB_BASE, b"\xf4")  # hlt: return sentinel
        self.heap = HEAP_BASE
        self.sizes: dict[int, int] = {}
        self.faults: list[str] = []
        self.import_calls: list[str] = []
        self.uc.hook_add(UC_HOOK_MEM_INVALID, self._fault)
        if runtime:
            self._setup_runtime()

    # ---- runtime shims -------------------------------------------------------------------
    def _setup_runtime(self):
        uc = self.uc
        # stub page layout: 0x100.. import stubs (one per import, `xor eax,eax; ret`),
        # 0x8000.. allocator method stubs (hooked)
        pe = self.pe
        pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"],
                                               pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT"]])
        stub = STUB_BASE + 0x100
        self.stub_names: dict[int, str] = {}
        for entry in list(getattr(pe, "DIRECTORY_ENTRY_IMPORT", [])) + list(getattr(pe, "DIRECTORY_ENTRY_DELAY_IMPORT", [])):
            for imp in entry.imports:
                uc.mem_write(stub, b"\x31\xc0\xc3")  # xor eax,eax; ret
                uc.mem_write(imp.address, struct.pack("<Q", stub))
                self.stub_names[stub] = f"{entry.dll.decode()}!{(imp.name or b'#').decode()}"
                stub += 4
        uc.hook_add(UC_HOOK_CODE, self._import_hook, begin=STUB_BASE + 0x100, end=stub)
        # allocator: object -> vtable of hooked stubs
        vt = STUB_BASE + 0x8000
        meth = STUB_BASE + 0x9000
        for i in range(N_ALLOC_SLOTS):
            uc.mem_write(vt + 8 * i, struct.pack("<Q", meth + 0x10 * i))
            uc.mem_write(meth + 0x10 * i, b"\xc3")
        uc.mem_write(self.base + ALLOCATOR_OBJ_RVA, struct.pack("<Q", vt))
        uc.hook_add(UC_HOOK_CODE, self._alloc_hook, begin=meth, end=meth + 0x10 * N_ALLOC_SLOTS)
        # TEB: gs:[0x58] -> TLS array; every slot -> block whose [0x68] is large
        uc.mem_map(TEB_BASE, 0x10000)
        tls_array = TEB_BASE + 0x1000
        tls_block = TEB_BASE + 0x2000
        uc.mem_write(TEB_BASE + 0x58, struct.pack("<Q", tls_array))
        for i in range(64):
            uc.mem_write(tls_array + 8 * i, struct.pack("<Q", tls_block))
        uc.mem_write(tls_block + 0x68, struct.pack("<i", 0x7FFFFFFF))
        uc.reg_write(UC_X86_REG_GS_BASE, TEB_BASE)

    def _import_hook(self, uc, addr, size, data):
        self.import_calls.append(self.stub_names.get(addr, hex(addr)))

    def _ret(self, value: int):
        rsp = self.uc.reg_read(UC_X86_REG_RSP)
        self.uc.reg_write(UC_X86_REG_RAX, value)
        # the `ret` stub executes after this hook; nothing else to do

    def _alloc_hook(self, uc, addr, size, data):
        slot = (addr - (STUB_BASE + 0x9000)) // 0x10
        rdx = uc.reg_read(UC_X86_REG_RDX)
        r8 = uc.reg_read(UC_X86_REG_R8)
        if slot == 1:                       # +0x08 alloc(this, size)
            self._ret(self.alloc(rdx))
        elif slot == 6:                     # +0x30 realloc(this, ptr, size)
            new = self.alloc(r8)
            old = self.sizes.get(rdx, 0)
            if rdx and old:
                self.uc.mem_write(new, bytes(self.uc.mem_read(rdx, min(old, r8))))
            self._ret(new)
        elif slot in (2, 4, 5):             # other alloc-like entry points (aligned etc.)
            self._ret(self.alloc(rdx if rdx < 0x10000000 else 0x100))
        else:                               # free and friends
            self._ret(0)

    def _fault(self, uc, access, address, size, value, data):
        self.faults.append(f"invalid memory access at 0x{address:x} (rip 0x{uc.reg_read(UC_X86_REG_RIP):x})")
        return False

    # ---- memory ---------------------------------------------------------------------------
    def alloc(self, n: int, data: bytes | None = None) -> int:
        n = max(int(n), 1)
        addr = self.heap
        self.heap += (n + 15) & ~15
        if self.heap > HEAP_BASE + HEAP_SIZE:
            raise MemoryError("emulator heap exhausted")
        self.sizes[addr] = n
        self.uc.mem_write(addr, data if data is not None else b"\0" * n)
        return addr

    def read(self, addr: int, n: int) -> bytes:
        return bytes(self.uc.mem_read(addr, n))

    def write(self, addr: int, data: bytes) -> None:
        self.uc.mem_write(addr, data)

    def u32(self, addr: int) -> int:
        return struct.unpack("<I", self.read(addr, 4))[0]

    def u64(self, addr: int) -> int:
        return struct.unpack("<Q", self.read(addr, 8))[0]

    # ---- calls ----------------------------------------------------------------------------
    def call(self, rva: int, *args: int, max_insns: int = 200_000_000) -> int:
        regs = [UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_R8, UC_X86_REG_R9]
        rsp = STACK_TOP - 0x1000
        stack_args = list(args[4:])
        rsp -= 8 * (len(stack_args) + 4)
        rsp &= ~0xF
        for i, v in enumerate(stack_args):
            self.uc.mem_write(rsp + 0x20 + 8 * i, struct.pack("<Q", v & 0xFFFFFFFFFFFFFFFF))
        rsp -= 8
        self.uc.mem_write(rsp, struct.pack("<Q", RET_SENTINEL))
        for r, v in zip(regs, args[:4]):
            self.uc.reg_write(r, v & 0xFFFFFFFFFFFFFFFF)
        self.uc.reg_write(UC_X86_REG_RSP, rsp)
        try:
            self.uc.emu_start(self.base + rva, RET_SENTINEL, count=max_insns)
        except UcError as e:
            rip = self.uc.reg_read(UC_X86_REG_RIP)
            raise RuntimeError(f"emulation failed at rip 0x{rip:x} (rva 0x{rip - self.base:x}): {e}; "
                               f"{'; '.join(self.faults[-3:])}") from e
        return self.uc.reg_read(UC_X86_REG_RAX)
