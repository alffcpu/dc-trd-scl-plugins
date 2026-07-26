# Disk image format reference

Brief notes on the two formats `zxdisk-core` reads and writes. See
[Development](DEVELOPMENT.md) for where this lives in the codebase.

**TRD** - a raw, headerless sector dump. 256-byte sectors, 16 per track.
Catalog in track 0 sectors 0-7 (128 x 16-byte entries); disk-info sector at
offset 0x800 (disk type at 0x8E3, TR-DOS id 0x10 at 0x8E7, free pointers,
label). Deleted entry = first name byte 0x01, data left intact. Files are
stored contiguously, allocated forward from the free pointer. Disk types
0x16-0x19 cover 160/320/640 KB, single/double sided.

**SCL** - `"SINCLAIR"` + file count + N x 14-byte descriptors + concatenated
file data + a trailing 4-byte little-endian additive checksum. No geometry,
no free-space map, no deleted files.

Sources: Sinclair Wiki (zxnet.co.uk), Kaitai `tr_dos_image`, ArchiveTeam File
Formats Wiki, World of Spectrum file-format FAQ, Double Commander
`wcxplugin.pas` and the Total Commander WCX SDK.
