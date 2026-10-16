    .data
    .p2align 3
atomic_counter:
    .xword  100

    .section .text,"ax",@progbits
    .p2align 2
    .global atomic_decrement
    .type atomic_decrement, %function
atomic_decrement:
    .cfi_startproc
    // x0 contains pointer to counter
.Lretry_loop:
    ldaxr   x1, [x0]            // Exclusive load with acquire
    sub     x2, x1, #58         // Subtract 58 -> expected 42
    stlxr   w3, x2, [x0]        // Store exclusive with release
    cbnz    w3, .Lretry_loop    // If collision, retry

    // Clear exclusive monitor just in case
    clrex
    // Return new value in w0
    mov     w0, w2
    ret
    .cfi_endproc
    .size atomic_decrement, .-atomic_decrement

    .global main
    .type main, %function
main:
    .cfi_startproc
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp

    // Full memory barrier before atomic operation
    dmb     ish

    adrp    x0, atomic_counter
    add     x0, x0, :lo12:atomic_counter

    bl      atomic_decrement
    // Expected result: 100 - 58 = 42

    ldp     x29, x30, [sp], #16
    ret
    .cfi_endproc
    .size main, .-main
    .section .note.GNU-stack,"",@progbits
