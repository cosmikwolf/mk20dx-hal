MEMORY
{
    FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 256K
    RAM   (rwx) : ORIGIN = 0x1FFF8000, LENGTH = 64K
}

/* Force .text to start after the flash configuration field (0x400-0x40F).
   cortex-m-rt uses _stext as the start of .text. By defining it here
   (not PROVIDE), we override the default and leave room for .flashconfig. */
_stext = 0x410;

/* Flash configuration field at exactly 0x400 */
SECTIONS
{
    .flashconfig 0x400 :
    {
        KEEP(*(.flashconfig))
    } > FLASH
}
INSERT AFTER .vector_table;
