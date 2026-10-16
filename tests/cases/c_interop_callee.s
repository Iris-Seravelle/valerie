    .section .text,"ax",@progbits
    .p2align 2
    .global vale_calc_with_c
    .type vale_calc_with_c, %function
vale_calc_with_c:
    .cfi_startproc
    // AAPCS64 Prologue
    stp     x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset 30, -24
    .cfi_offset 29, -32
    mov     x29, sp
    .cfi_def_cfa_register 29

    // Save callee-saved registers x19, x20
    stp     x19, x20, [sp, #16]

    // Accumulate first 8 GPR arguments: w0..w7
    add     w0, w0, w1
    add     w0, w0, w2
    add     w0, w0, w3
    add     w0, w0, w4
    add     w0, w0, w5
    add     w0, w0, w6
    add     w0, w0, w7

    // 9th integer argument was passed on stack by caller!
    // Caller's SP was [FP + 32]. The 9th argument is at [FP + 32].
    ldr     w8, [x29, #32]
    add     w0, w0, w8         // Total sum: 1+2+3+4+5+6+7+8+9 = 45

    mov     w19, w0             // Preserve sum in callee-saved w19

    // Call back into C: c_callback_multiplier(w0, w1)
    // Setup args: w0 = 45, w1 = 2
    mov     w0, w19
    mov     w1, #2
    bl      c_callback_multiplier
    // Returned in w0 = 90

    // Subtract 48 -> expected 42
    sub     w0, w0, #48

    // Epilogue
    ldp     x19, x20, [sp, #16]
    ldp     x29, x30, [sp], #32
    .cfi_def_cfa 31, 0
    ret
    .cfi_endproc
    .size vale_calc_with_c, .-vale_calc_with_c
    .section .note.GNU-stack,"",@progbits
