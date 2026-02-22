MEMORY
{
    FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 128K
    RAM   (rwx) : ORIGIN = 0x1FFFE000, LENGTH = 16K
}

/* Flash configuration field — must be placed at 0x400 */
SECTIONS
{
    .flashconfig 0x400 :
    {
        KEEP(*(.flashconfig))
    } > FLASH
}
INSERT BEFORE .text;
