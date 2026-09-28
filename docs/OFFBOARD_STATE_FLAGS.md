# Off-board state flag correction

The user reported jumps immediately playing a landing animation, repeated
landing animations and rising motion after walking off a ledge. The preceding
run's stderr contained no airborne bail report; this alone does not establish
whether the airborne state was entered.

Static evidence: TU3 dump `default_82000000_011B0000.bin`, SHA-256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
The generated instruction listing for `82D8ADE8` contains:

- `82D8B8D4`: `lhz r11,2484(r4)`, followed by a low-bit test.
- `82D8B490` and `82D8B5F0`: `lbz r11,2484(r4)`, followed by a low-bit test.

On this big-endian target these test full-word masks `0x00010000` and
`0x01000000`, respectively. The port incorrectly used `0x00000001` for all
three. Bit16 is published from Offboard328, the airborne continuation flag;
bit0 is a separate animation attribute. BipedAir therefore incorrectly returned
to BipedGround while its airborne flag was still set.

Corrected all three masks. Repeated premature ground entry is a plausible
cause of the reported animation loop; gameplay confirmation remains with the
user. Build only, no tests run. Bounded bail diagnostics also cover BipedGround
so any remaining turning-related failure can be captured.
