# Notices and credits

rac-rs is licensed under GPL-3.0-or-later (see LICENSE). It builds on the published knowledge and work listed below.

## Contributors

- **DeSynkro** (https://github.com/DeSynkro): the Linux port (pull request #38, v0.4.1): building and running the game on Linux, Linux gamepads, the Linux run and crash logs and the exact maths that give Linux the same game memory as Windows. Contributed under GPL-3.0-or-later like the rest of rac-rs. This fork adds local save dates, the boot logo skip, the Camera Reversed stick fix, the PlayStation / Xbox / Switch button artwork and the title Graphics page.

## rac1-decomp

Several level-file readers began as Rust ports of the Python extraction tools in rac1-decomp: terrain, ties, shrubs, sky, level, table-of-contents and collision. The rac1-decomp tools are released under this licence (its LICENSE file, quoted in full; `src/libgcc/` refers to that repository):

```
MIT License

Copyright (c) 2026 Kryštof "Lynder063" Malinda

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

The files in `src/libgcc/` that
come from GCC keep their original license (GPL v2 with the libgcc linking
exception), as stated in each file's header.

*Ratchet & Clank* is a trademark of Sony Interactive Entertainment. This
project is not affiliated with or endorsed by Sony or Insomniac Games.
```

## Wrench

The Wrench modding tools and their documentation (the Ratchet & Clank modding community) describe the game's file formats. rac1-decomp's readers follow them, and so do ours.

## PCSX2

The sound (SPU2), graphics (GS/VU) and CPU behaviour in this project was written against the PCSX2 emulator as a hardware-behaviour reference. PCSX2 is licensed under GPL-3.0-or-later; rac-rs uses the same licence.

## Hardware documentation

psx-spx (Martin Korth / nocash) documents the SPU ADPCM and Gaussian interpolation tables. These are properties of the PlayStation hardware.

## Rust libraries

rac-rs uses crates from crates.io, including wgpu, winit, cpal, rayon, bytemuck, pollster and xxhash-rust, under the MIT, Apache-2.0, Zlib, BSD, ISC and BSL-1.0 licences. Cargo downloads them at build time. Their licence texts ship with each crate.

## OmniLoader mod loader (bundled/omniloader.dll) and Arsenal Mastery

bundled/omniloader.dll is OmniLoader, this project's own mod loader (MIT OR Apache-2.0), written from the MIT-licensed OpenRAC SDK documentation. Built into it: the Luau script engine (MIT; Roblox Corporation and Lua.org, PUC-Rio) through the mlua crate (MIT), and serde / serde_json (MIT OR Apache-2.0). bundled/mods/arsenal-mastery is a mod of this project with its own LICENSE file.

OmniLoader's mod format (the `omni-mod.json` manifest, which also accepts OpenRAC's `openrac-mod.json`), its Luau API and its sandbox rules follow the OpenRAC SDK 1.3 (https://github.com/OpenRAC/OpenRAC), written from its public documentation and data. No code of OpenRAC's GPL-3.0 game port was read or copied. The OpenRAC SDK licence, copied unchanged:

```
MIT License

Copyright (c) 2026 the OpenRAC contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

The Luau licence, copied unchanged from the luau0-src crate:

```
MIT License

Copyright (c) 2022-2025 Aleksandr Orlenko (luau-src crate)
Copyright (c) 2019-2024 Roblox Corporation
Copyright (c) 1994–2019 Lua.org, PUC-Rio.

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to do
so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Microsoft DirectX Shader Compiler (dxcompiler.dll)

On Windows the game can use Microsoft's DirectX Shader Compiler (DXC) to translate its shaders when you choose the DirectX 12 renderer
(Options -> Graphics -> Screen -> Renderer). It makes the first start about 7 s faster than the built-in fallback (FXC). The game works
without it.

rac-rs does not contain or ship DXC. The setup (setup.ps1) downloads Microsoft's own release archive onto your PC and copies only
`bin\x64\dxcompiler.dll` next to `rac-rs.exe`:

- Project: https://github.com/microsoft/DirectXShaderCompiler
- Release: v1.9.2609 (2026-09-29), file `dxc_2026_09_29.zip`, SHA-256 `AD31B1FC8443175D204F77A611FDB3EF2EC42759BDC2F1167368DE24A4A7E7F1`
  (setup checks this; if the download fails or the checksum differs, the game simply uses FXC)
- `dxcompiler.dll` (x64) SHA-256 `0689E2FEE0A1C9DF8662A59680469F24E2A3AD7CE6BBBCC9284F7A83172BFC59`

The licence texts below are copied unchanged from that archive (LICENSE-MS.txt, LICENCE-MIT.txt, LICENSE-LLVM.txt).

### LICENSE-MS.txt (Microsoft software licence terms for the released binaries)

MICROSOFT SOFTWARE LICENSE TERMS

MICROSOFT DIRECTX SHADER COMPILER

These license terms are an agreement between you and Microsoft
Corporation (or one of its affiliates). They apply to the software named
above and any Microsoft services or software updates (except to the
extent such services or updates are accompanied by new or additional
terms, in which case those different terms apply prospectively and do
not alter your or Microsoft’s rights relating to pre-updated software or
services). IF YOU COMPLY WITH THESE LICENSE TERMS, YOU HAVE THE RIGHTS
BELOW. BY USING THE SOFTWARE, YOU ACCEPT THESE TERMS.

INSTALLATION AND USE RIGHTS. 

General. Subject to the terms of this agreement, you may install and use any number of copies of the software, and solely for use on Windows.

Included Microsoft Applications. The software may include other Microsoft applications. These license terms apply to those included applications, if any, unless other license terms are provided with the other Microsoft applications.

Microsoft Platforms. The software may include components from Microsoft Windows. These components are governed by separate agreements and their own product support policies, as described in the license terms found in the installation directory for that component or in the “Licenses” folder accompanying the software.

Third Party Components. The software may include third party components with separate legal notices or governed by other agreements, as may be described in the ThirdPartyNotices file(s) accompanying the software.

DATA. 

Data Collection. The software may collect information about you and your use of the software, and send that to Microsoft. Microsoft may use this information to provide services and improve our products and services. You may opt-out of many of these scenarios, but not all, as described in the product documentation.  There are also some features in the software that may enable you to collect data from users of your applications. If you use these features to enable data collection in your applications, you must comply with applicable law, including providing appropriate notices to users of your applications. You can learn more about data collection and use in the help documentation and the privacy statement at https://aka.ms/privacy. Your use of the software operates as your consent to these practices.

Processing of Personal Data. To the extent Microsoft is a processor or subprocessor of personal data in connection with the software, Microsoft makes the commitments in the European Union General Data Protection Regulation Terms of the Online Services Terms to all customers effective May 25, 2018, at https://docs.microsoft.com/en-us/legal/gdpr.

DISTRIBUTABLE CODE. The software may contain code you are permitted to distribute (i.e. make available for third parties) in applications you develop, as described in this Section. 

Distribution Rights. The code and test files described below are distributable if included with the software. 

Distributables. You may copy and distribute the object code form of the software listed in the distributables file list in the software; and

Third Party Distribution. You may permit distributors of your applications to copy and distribute any of this distributable code you elect to distribute with your applications.

Distribution Requirements. For any code you distribute, you must:

add significant primary functionality to it in your applications; 

i.  require distributors and external end users to agree to terms that protect it and Microsoft at least as much as this agreement; and

ii. indemnify, defend, and hold harmless Microsoft from any claims, including attorneys’ fees, related to the distribution or use of your applications, except to the extent that any claim is based solely on the unmodified distributable code.

Distribution Restrictions. You may not:

use Microsoft’s trademarks or trade dress in your application in any way that suggests your application comes from or is endorsed by Microsoft; or modify or distribute the source code of any distributable code so that any part of it becomes subject to any license that requires that the distributable code, any other part of the software, or any of Microsoft’s other intellectual property be disclosed or distributed in source code form, or that others have the right to modify it.

SCOPE OF LICENSE. The software is licensed, not sold. Microsoft reserves all other rights. Unless applicable law gives you more rights despite this limitation, you will not (and have no right to):

work around any technical limitations in the software that only allow you to use it in certain ways;

reverse engineer, decompile or disassemble the software, or otherwise attempt to derive the source code for the software, except and to the extent required by third party licensing terms governing use of certain open source components that may be included in the software;

remove, minimize, block, or modify any notices of Microsoft or its suppliers in the software;

use the software in any way that is against the law or to create or propagate malware; or

share, publish, distribute, or lease the software (except for any distributable code, subject to the terms above), provide the software as a stand-alone offering for others to use, or transfer the software or this agreement to any third party.

EXPORT RESTRICTIONS. You must comply with all domestic and international export laws and regulations that apply to the software, which include restrictions on destinations, end users, and end use. For further information on export restrictions, visit https://aka.ms/exporting.

SUPPORT SERVICES. Microsoft is not obligated under this agreement to provide any support services for the software. Any support provided is “as is”, “with all faults”, and without warranty of any kind.

UPDATES. The software may periodically check for updates, and download and install them for you. You may obtain updates only from Microsoft or authorized sources. Microsoft may need to update your system to provide you with updates. You agree to receive these automatic updates without any additional notice. Updates may not include or support all existing software features, services, or peripheral devices.

ENTIRE AGREEMENT. This agreement, and any other terms Microsoft may provide for supplements, updates, or third-party applications, is the entire agreement for the software.

APPLICABLE LAW AND PLACE TO RESOLVE DISPUTES. If you acquired the software in the United States or Canada, the laws of the state or province where you live (or, if a business, where your principal place of business is located) govern the interpretation of this agreement, claims for its breach, and all other claims (including consumer protection, unfair competition, and tort claims), regardless of conflict of laws principles. If you acquired the software in any other country, its laws apply. If U.S. federal jurisdiction exists, you and Microsoft consent to exclusive jurisdiction and venue in the federal court in King County, Washington for all disputes heard in court. If not, you and Microsoft consent to exclusive jurisdiction and venue in the Superior Court of King County, Washington for all disputes heard in court.

CONSUMER RIGHTS; REGIONAL VARIATIONS. This agreement describes certain legal rights. You may have other rights, including consumer rights, under the laws of your state or country. Separate and apart from your relationship with Microsoft, you may also have rights with respect to the party from which you acquired the software. This agreement does not change those other rights if the laws of your state or country do not permit it to do so. For example, if you acquired the software in one of the below regions, or mandatory country law applies, then the following provisions apply to you:

a.  Australia. You have statutory guarantees under the Australian
    Consumer Law and nothing in this agreement is intended to affect
    those rights.

b.  Canada. If you acquired this software in Canada, you may stop
    receiving updates by turning off the automatic update feature,
    disconnecting your device from the Internet (if and when you
    re-connect to the Internet, however, the software will resume
    checking for and installing updates), or uninstalling the software.
    The product documentation, if any, may also specify how to turn off
    updates for your specific device or software.

c.  Germany and Austria.

  i. Warranty. The properly licensed software will perform substantially
  as described in any Microsoft materials that accompany the software.
  However, Microsoft gives no contractual guarantee in relation to the
  licensed software.

  ii. Limitation of Liability. In case of intentional conduct, gross
  negligence, claims based on the Product Liability Act, as well as, in
  case of death or personal or physical injury, Microsoft is liable
  according to the statutory law.

Subject to the foregoing clause ii., Microsoft will only be liable for slight negligence if Microsoft is in breach of such material contractual obligations, the fulfillment of which facilitate the due performance of this agreement, the breach of which would endanger the purpose of this agreement and the compliance with which a party may constantly trust in (so-called "cardinal obligations"). In other cases of slight negligence, Microsoft will not be liable for slight negligence.

DISCLAIMER OF WARRANTY. THE SOFTWARE IS LICENSED “AS IS.” YOU BEAR THE RISK OF USING IT. MICROSOFT GIVES NO EXPRESS WARRANTIES, GUARANTEES, OR CONDITIONS. TO THE EXTENT PERMITTED UNDER APPLICABLE LAWS, MICROSOFT EXCLUDES ALL IMPLIED WARRANTIES, INCLUDING MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, AND NON-INFRINGEMENT.

LIMITATION ON AND EXCLUSION OF DAMAGES. IF YOU HAVE ANY BASIS FOR RECOVERING DAMAGES DESPITE THE PRECEDING DISCLAIMER OF WARRANTY, YOU CAN RECOVER FROM MICROSOFT AND ITS SUPPLIERS ONLY DIRECT DAMAGES UP TO U.S. $5.00. YOU CANNOT RECOVER ANY OTHER DAMAGES, INCLUDING CONSEQUENTIAL, LOST PROFITS, SPECIAL, INDIRECT, OR INCIDENTAL DAMAGES.

This limitation applies to (a) anything related to the software,
services, content (including code) on third party Internet sites, or
third party applications; and (b) claims for breach of contract,
warranty, guarantee, or condition; strict liability, negligence, or
other tort; or any other claim; in each case to the extent permitted by
applicable law.

It also applies even if Microsoft knew or should have known about the
possibility of the damages. The above limitation or exclusion may not
apply to you because your state, province, or country may not allow the
exclusion or limitation of incidental, consequential, or other damages.

Please note: As this software is distributed in Canada, some of the
clauses in this agreement are provided below in French.

Remarque: Ce logiciel étant distribué au Canada, certaines des clauses
dans ce contrat sont fournies ci-dessous en français.

EXONÉRATION DE GARANTIE. Le logiciel visé par une licence est offert «
tel quel ». Toute utilisation de ce logiciel est à votre seule risque et
péril. Microsoft n’accorde aucune autre garantie expresse. Vous pouvez
bénéficier de droits additionnels en vertu du droit local sur la
protection des consommateurs, que ce contrat ne peut modifier. La ou
elles sont permises par le droit locale, les garanties implicites de
qualité marchande, d’adéquation à un usage particulier et d’absence de
contrefaçon sont exclues.

LIMITATION DES DOMMAGES-INTÉRÊTS ET EXCLUSION DE RESPONSABILITÉ POUR LES
DOMMAGES. Vous pouvez obtenir de Microsoft et de ses fournisseurs une
indemnisation en cas de dommages directs uniquement à hauteur de 5,00 $
US. Vous ne pouvez prétendre à aucune indemnisation pour les autres
dommages, y compris les dommages spéciaux, indirects ou accessoires et
pertes de bénéfices.

Cette limitation concerne:

• tout ce qui est relié au logiciel, aux services ou au contenu (y
compris le code) figurant sur des sites Internet tiers ou dans des
programmes tiers; et

• les réclamations au titre de violation de contrat ou de garantie, ou
au titre de responsabilité stricte, de négligence ou d’une autre faute
dans la limite autorisée par la loi en vigueur.

Elle s’applique également, même si Microsoft connaissait ou devrait
connaître l’éventualité d’un tel dommage. Si votre pays n’autorise pas
l’exclusion ou la limitation de responsabilité pour les dommages
indirects, accessoires ou de quelque nature que ce soit, il se peut que
la limitation ou l’exclusion ci-dessus ne s’appliquera pas à votre
égard.

EFFET JURIDIQUE. Le présent contrat décrit certains droits juridiques.
Vous pourriez avoir d’autres droits prévus par les lois de votre pays.
Le présent contrat ne modifie pas les droits que vous confèrent les lois
de votre pays si celles-ci ne le permettent pas.

### LICENCE-MIT.txt

Copyright (c) Microsoft Corporation.

MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED *AS IS*, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

### LICENSE-LLVM.txt

==============================================================================
LLVM Release License
==============================================================================
University of Illinois/NCSA
Open Source License

Copyright (c) 2003-2015 University of Illinois at Urbana-Champaign.
All rights reserved.

Developed by:

    LLVM Team

    University of Illinois at Urbana-Champaign

    http://llvm.org

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal with
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to do
so, subject to the following conditions:

    * Redistributions of source code must retain the above copyright notice,
      this list of conditions and the following disclaimers.

    * Redistributions in binary form must reproduce the above copyright notice,
      this list of conditions and the following disclaimers in the
      documentation and/or other materials provided with the distribution.

    * Neither the names of the LLVM Team, University of Illinois at
      Urbana-Champaign, nor the names of its contributors may be used to
      endorse or promote products derived from this Software without specific
      prior written permission.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
CONTRIBUTORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS WITH THE
SOFTWARE.
