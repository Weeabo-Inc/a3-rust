# Binary overview: arma3_x64.exe (and arma3server_x64.exe)

Game build 2.22.0.154103 (`FileVersion 2.22`). All addresses are **RVAs**; add the image base
`0x140000000` for the VA that Ghidra shows (RVA `0x2e2380` = VA `0x1402e2380`).
Source of facts: PE headers (pefile), strings, Ghidra auto-analysis. Confidence: high unless noted.

## Build and compiler

| | arma3_x64.exe | arma3server_x64.exe |
|---|---|---|
| Size | 39,206,208 B | 32,131,832 B |
| Image base | 0x140000000 | 0x140000000 |
| Entry point (RVA) | 0x2637310 (in `.bind`, SteamStub) | 0x1569c6c (in `.text`) |
| PE timestamp | 1791463812 | 1791464571 |
| PDB path (CodeView) | `O:\Arma3\Arma3Retail_DX11_x64.pdb` | `O:\Arma3\Arma3Retail_Server_x64.pdb` |
| Linker | MSVC 14.43 (Visual Studio 2022 17.13); Rich header tools build 34808 | same |
| DllCharacteristics | 0x8160 (high-entropy VA, dynamic base, NX, terminal-server aware) | same |
| TLS callbacks | yes (VA 0x141a778e8) | yes |
| Authenticode | 12,024 B certificate table | 12,024 B |
| `.pdata` functions | 107,538 RUNTIME_FUNCTION starts | — |

No PDB ships with the game; there are no debug symbols. RTTI is present (5,179 type descriptors,
see `rtti-classes.md`), and many functions are identifiable by their log/assert strings
(`"ActorHolderPhysx3::AddForce"`, ...).

## Sections (arma3_x64.exe)

| Section | RVA | Virtual size | Raw size | Entropy | Notes |
|---|---|---|---|---|---|
| `.text` | 0x1000 | 0x1a6f5a8 | 0x1a6f600 | 6.46 | code, **not encrypted** |
| `.rdata` | 0x1a71000 | 0x615ef0 | 0x616000 | 5.78 | strings, vtables, RTTI COLs |
| `.data` | 0x2087000 | 0x1e44e8 | 0xd9800 | 5.27 | globals, RTTI type descriptors |
| `.pdata` | 0x226c000 | 0x13b0d8 | 0x13b200 | 6.89 | unwind info |
| `_RDATA` | 0x23a8000 | 0x13b10 | 0x13c00 | 5.50 | |
| `.rsrc` | 0x23bc000 | 0x217540 | 0x217600 | 6.76 | icons, version |
| `.reloc` | 0x25d4000 | 0x62508 | 0x62600 | 5.46 | |
| `.bind` | 0x2637000 | 0x39248 | 0x39248 | 7.96 | SteamStub DRM wrapper |

The server binary has the same layout without `.bind` (`.text` 0x170015c bytes).

## Protection

- **SteamStub (Steam DRM wrapper)** on the client only: the extra `.bind` section (high entropy,
  executable) holds the stub and the PE entry point points into it. The stub checks Steam and
  then jumps to the real CRT entry in `.text`.
- The `.text` section is **not encrypted** on disk (entropy 6.46, normal x64 code; Ghidra
  disassembles and decompiles it directly from the file). Static analysis needs no unpacking.
  Running the client still needs Steam.
- `arma3server_x64.exe` has no `.bind` section: no SteamStub. It is the easier target for
  network RE and contains no renderer.
- BattlEye is external (`BattlEye\BEClient_x64.dll` / `BEServer_x64.dll`, loaded at runtime,
  strings at RVA 0x1b40d50 / 0x1b416e0). It is not linked into the exe.
- No commercial packer/virtualiser (VMProtect/Themida/Denuvo) signatures found: no such section
  names, normal import table, normal `.pdata` coverage.

## Imports by DLL (arma3_x64.exe)

Static imports:

| DLL | Functions | Purpose |
|---|---|---|
| `d3d11.dll` | D3D11CreateDevice, D3D11CreateDeviceAndSwapChain | renderer (DX11 only) |
| `dxgi.dll` | CreateDXGIFactory | swap chain / adapters |
| `d3d10.dll` | D3D10CreateBlob | |
| `d3dx11_43.dll` | D3DX11CreateTextureFromMemory, D3DX11GetImageInfoFromMemory, D3DX11CompileFromMemory, D3DX11SaveTextureToFileA | texture load, runtime shader compile, screenshots |
| `d3dx10_43.dll` | D3DXMatrixMultiply, D3DXFloat32To16Array | math |
| `D3DCOMPILER_43.dll` | D3DDisassemble, D3DStripShader | |
| `XAudio2_8.dll` | ordinals 4, 5, 6 (XAudio2Create, ...) | main audio output |
| `WINMM.dll` | waveIn*, timeGetTime, timeBeginPeriod | microphone capture (VoN), timers |
| `XINPUT1_3.dll` | ordinals 2, 3 | gamepads |
| `DINPUT8.dll` | DirectInput8Create | joysticks |
| `WS2_32.dll` | socket, bind, sendto, recvfrom, select, getaddrinfo, ... (29) | UDP networking |
| `IPHLPAPI.DLL` | GetAdaptersAddresses, GetAdaptersInfo, GetBestRoute, GetIpAddrTable | local address discovery |
| `DNSAPI.dll` | DnsQuery_W, DnsFree | server-browser DNS lookups |
| `WINTRUST.dll` | WinVerifyTrust | signature check of loaded DLLs (extensions) |
| `AVRT.dll` | AvSetMmThreadCharacteristics* | MMCSS for audio threads |
| `WindowsCodecs.dll`, `MSIMG32.dll`, `ole32`, `OLEAUT32`, `SHELL32`, `SHLWAPI`, `VERSION`, `ntdll`, `KERNEL32` (239), `USER32` (97), `GDI32` (25), `ADVAPI32` (29) | | OS |

Delay-loaded imports (loaded only when used):

| DLL | Functions | Library / version shipped in `Dll\x64` |
|---|---|---|
| `PhysX_64.dll`, `PhysXFoundation_64.dll`, `PhysXCooking_64.dll` | 31 / 33 / PxCreateCooking | NVIDIA PhysX **4.1.1.0** (engine wrappers are still named `*Physx3*`) |
| `RTDynamics_64.dll` | 362 | RotorLib/RTDynamics advanced helicopter flight model |
| `OpenAL32.dll` | 35 | OpenAL 1.1 (fallback audio / VoN capture; "Missing OpenAL32.DLL" message) |
| `libcurl.dll` | curl_easy_*, curl_multi_* (17) | libcurl **8.21.0** (HTTP downloads) |
| `steam_api64.dll` | SteamAPI_*, SteamInternal_*, SteamGameServer_* (20) | Steamworks 09.31.86.04 |
| `Tobii.EyeX.Client.dll` | 26 | Tobii EyeX 1.13.2 |
| `GFSDK_SSAO_D3D11.win64.dll` | GFSDK_SSAO_CreateContext_D3D11 | NVIDIA HBAO+ 3.0 |

The server imports the same set minus D3D/DXGI/XAudio2/XInput/DInput/Tobii/HBAO, uses
`WSOCK32.dll` (23) + `WS2_32.dll` (4), and keeps PhysX, OpenAL, libcurl and steam_api64.

## Embedded third-party code (statically linked)

| Library | Evidence (string RVA) | Confidence |
|---|---|---|
| zlib **1.2.13.1-motley** | version string 0x1aa4d60; `Compression.Zlib` 0x1ab63d8 | high |
| LZO Professional | `"LZO Professional"` 0x1a975b8 | high (LZO1X used for PBO/ODOL/WRP compression — see Phase 1) |
| OpenSSL (1.0.x era) | licence text "Copyright (c) 1998-2016 The OpenSSL Project" 0x1b34d00; `OpenSSL_license.txt` ships | high |
| Botan (C++ crypto) | `Botan::RSA_Private_Operation::decrypt` 0x1abf000, PKCS#1/EMSA names 0x1ab6410.. | high — likely used for bisign/bikey RSA verification |
| Opus codec | "Opus Codec: ..." 0x1a94df8, Xiph/Skype copyright 0x1a94d20 | high (VoN voice) |
| Speex codec | `"Speex codec"` 0x1aabc58 | high (legacy VoN) |
| Boost.Regex | "Attempt to access an uninitialized boost::match_results<> class." 0x1b4e390 | high |
| rapidjson | RTTI `rapidjson::GenericValue<...>` | high |
| Intel TBB malloc | `tbb4malloc_bi` 0x1a79498 (loads `Dll\tbb4malloc_bi_x64.dll`, TBB 2017) | high |
| Enfusion (`enf::`) thread helpers | RTTI `enf::CThread::ParamsMethod<...>` | high |
| JNI scripting bridge | `Lcom/bistudio/JNIScripting/RVEngine$GameNetObject;` 0x1accf08 | medium (dormant Java scripting support) |
| SMAA / FXAA | `readme_smaa.txt`, `readme_fxaa.txt` ship with the game | medium |

Not found: Lua, FreeType (fonts are the engine's own), libogg/libvorbis strings (Ogg Vorbis
decoding exists — `.ogg` is the main sound format — but no version banner; the decoder is
probably stb_vorbis-style or stripped), LZ4, zstd, FMOD, Havok.

## Engine identity strings

`[['Real Virtuality','(C) 2012 Bohemia Interactive','All rights reserved']]` at 0x1b55280.
Script command help texts are compiled in (descriptions, examples, `since` versions), see
`sqf-command-table.md`.
