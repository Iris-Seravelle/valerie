#include <stdint.h>
#include <stdio.h>
#include <assert.h>

// A composite struct according to AAPCS64
struct AbiStruct {
    int64_t a;
    double b;
    int32_t c;
};

// Declared in Vale assembly / emitted code:
// Takes 9 arguments (tests first 8 GPRs/FPRs + 9th on stack)
extern int32_t vale_calc_with_c(
    int32_t g1, int32_t g2, int32_t g3, int32_t g4,
    int32_t g5, int32_t g6, int32_t g7, int32_t g8,
    int32_t g9_on_stack,
    double f1, double f2
);

// C function called from Vale assembly
int32_t c_callback_multiplier(int32_t x, int32_t y) {
    return x * y;
}

int main(void) {
    // Calling Vale assembly function from C driver
    int32_t result = vale_calc_with_c(
        1, 2, 3, 4,
        5, 6, 7, 8,
        9,       // passed on stack
        1.5, 2.5 // passed in d0, d1
    );

    // Expected calculation in Vale assembly:
    // (g1 + g2 + g3 + g4 + g5 + g6 + g7 + g8 + g9) = 45
    // called c_callback_multiplier(45, 2) = 90
    // minus 48 = 42
    assert(result == 42);
    return result;
}
