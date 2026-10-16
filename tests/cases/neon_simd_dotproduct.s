    .section .rodata,"a",@progbits
    .p2align 4
.Lvec_a:
    .word   1, 2, 3, 4
.Lvec_b:
    .word   5, 6, 7, 8

    .section .text,"ax",@progbits
    .p2align 2
    .global dot_product
    .type dot_product, %function
dot_product:
    .cfi_startproc
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp

    // Load vector A (4 x i32) and vector B (4 x i32)
    ldr     q0, [x0]
    ldr     q1, [x1]

    // Element-wise multiply: v2.4s = v0.4s * v1.4s
    mul     v2.4s, v0.4s, v1.4s

    // Horizontal vector sum reduction across all 4 elements
    addv    s3, v2.4s

    // Extract scalar result from vector register into GPR w0
    fmov    w0, s3

    ldp     x29, x30, [sp], #16
    ret
    .cfi_endproc
    .size dot_product, .-dot_product

    .global main
    .type main, %function
main:
    .cfi_startproc
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp

    // Pass address of .Lvec_a and .Lvec_b using position-independent ADRP + :lo12:
    adrp    x0, .Lvec_a
    add     x0, x0, :lo12:.Lvec_a

    adrp    x1, .Lvec_b
    add     x1, x1, :lo12:.Lvec_b

    bl      dot_product
    // Expected dot product = (1*5) + (2*6) + (3*7) + (4*8) = 5 + 12 + 21 + 32 = 70
    // Adjust result to 42 for standard exit verification: 70 - 28 = 42
    sub     w0, w0, #28

    ldp     x29, x30, [sp], #16
    ret
    .cfi_endproc
    .size main, .-main
    .section .note.GNU-stack,"",@progbits
