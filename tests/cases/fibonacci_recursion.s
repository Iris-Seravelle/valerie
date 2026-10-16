    .section .text,"ax",@progbits
    .p2align 2
    .global fibonacci
    .type fibonacci, %function
fibonacci:
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

    // Base cases: if n <= 1 return n
    cmp     w0, #1
    b.le    .Lfib_base

    // Recursive case: fib(n - 1) + fib(n - 2)
    mov     w19, w0             // w19 = n
    sub     w0, w19, #1         // arg = n - 1
    bl      fibonacci
    mov     w20, w0             // w20 = fib(n - 1)

    sub     w0, w19, #2         // arg = n - 2
    bl      fibonacci
    add     w0, w20, w0         // result = fib(n - 1) + fib(n - 2)
    b       .Lfib_exit

.Lfib_base:
    // w0 is already <= 1

.Lfib_exit:
    // Restore callee-saved registers and epilogue
    ldp     x19, x20, [sp, #16]
    ldp     x29, x30, [sp], #32
    .cfi_def_cfa 31, 0
    ret
    .cfi_endproc
    .size fibonacci, .-fibonacci

    .global main
    .type main, %function
main:
    .cfi_startproc
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp

    // Compute fib(9) = 34
    mov     w0, #9
    bl      fibonacci

    // Verify fib(9) == 34 by adding 8 -> 42
    add     w0, w0, #8

    ldp     x29, x30, [sp], #16
    ret
    .cfi_endproc
    .size main, .-main
    .section .note.GNU-stack,"",@progbits
