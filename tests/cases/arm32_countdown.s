    .syntax unified
    .arch armv7-a
    .arm
    .text
    .global _start
_start:
    // Arm32 AAPCS32 recursive countdown function test
    mov     r0, #10
    bl      factorial_base

    // Expected: 10 + 32 = 42
    add     r0, r0, #32

    // Linux Arm32 sys_exit (svc #0 with r7 = 1)
    mov     r7, #1
    svc     #0

factorial_base:
    push    {r4, lr}
    mov     r4, r0
    cmp     r4, #0
    ble     .Ldone
    sub     r0, r4, #1
    bl      factorial_base
.Ldone:
    mov     r0, r4
    pop     {r4, pc}
