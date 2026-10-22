/* Mach VM consumer: allocation, protection and error contracts.
 * No headers: the fixture declares only the ABI it exercises. */
typedef unsigned int kern_return_t;
typedef unsigned long long u64;
typedef unsigned int u32;

extern int puts(const char *);

extern kern_return_t mach_vm_allocate(u32 task, u64 *address, u64 size, int flags);
extern kern_return_t mach_vm_deallocate(u32 task, u64 address, u64 size);
extern kern_return_t mach_vm_protect(u32 task, u64 address, u64 size, int set_maximum,
                                     int new_protection);

#define TASK 0x103u
#define PAGE 4096ull
#define ANYWHERE 1
#define PROT_READ 1
#define PROT_WRITE 2
#define PROT_EXEC 4

#define KERN_SUCCESS 0
#define KERN_INVALID_ADDRESS 1
#define KERN_NO_SPACE 3
#define KERN_INVALID_ARGUMENT 4

static int failures;

static void expect(int step, int actual, int wanted) {
    if (actual != wanted) {
        failures = failures + 1 + step * 1000;
    }
}

int main(void) {
    u64 size = PAGE * 4;
    u64 address = 0;
    int step = 0;

    /* An anywhere allocation must return a page-aligned writable range. */
    expect(step++, mach_vm_allocate(TASK, &address, size, ANYWHERE), KERN_SUCCESS);
    if (address == 0 || (address & (PAGE - 1)) != 0) {
        puts("mach_vm:allocation is not page-aligned");
        return 1;
    }
    unsigned char *bytes = (unsigned char *)address;
    for (u64 i = 0; i < size; i++) {
        bytes[i] = (unsigned char)(i & 0xff);
    }
    int intact = 1;
    for (u64 i = 0; i < size; i++) {
        if (bytes[i] != (unsigned char)(i & 0xff)) {
            intact = 0;
        }
    }
    expect(step++, intact, 1);

    /* A fixed allocation at a live range has no room. */
    expect(step++, mach_vm_allocate(TASK, &address, size, 0), KERN_NO_SPACE);

    /* Protection changes are accepted and data survives the round trip. */
    expect(step++, mach_vm_protect(TASK, address, size, 0, PROT_READ), KERN_SUCCESS);
    expect(step++, mach_vm_protect(TASK, address, size, 0, PROT_READ | PROT_WRITE),
           KERN_SUCCESS);
    intact = 1;
    for (u64 i = 0; i < size; i++) {
        if (bytes[i] != (unsigned char)(i & 0xff)) {
            intact = 0;
        }
    }
    expect(step++, intact, 1);

    /* Invalid requests are refused without touching the mapping. */
    expect(step++, mach_vm_protect(TASK, address, size, 0, PROT_WRITE | PROT_EXEC),
           KERN_INVALID_ARGUMENT);
    expect(step++, mach_vm_protect(TASK, address + PAGE * 8, size, 0, PROT_READ),
           KERN_INVALID_ADDRESS);
    expect(step++, mach_vm_protect(TASK, 0, size, 0, PROT_READ), KERN_INVALID_ADDRESS);
    expect(step++, mach_vm_deallocate(TASK, address + PAGE, size), KERN_INVALID_ADDRESS);
    expect(step++, mach_vm_deallocate(TASK + 1, address, size), KERN_INVALID_ARGUMENT);
    expect(step++, mach_vm_allocate(TASK, &address, 0, ANYWHERE), KERN_INVALID_ARGUMENT);
    expect(step++, mach_vm_allocate(TASK, &address, 1ull << 40, ANYWHERE), KERN_NO_SPACE);
    expect(step++, mach_vm_allocate(TASK, &address, size, 2), KERN_INVALID_ARGUMENT);

    /* Release, then the same fixed address can be reserved again. */
    expect(step++, mach_vm_deallocate(TASK, address, size), KERN_SUCCESS);
    expect(step++, mach_vm_deallocate(TASK, address, size), KERN_INVALID_ADDRESS);
    u64 fixed = address;
    expect(step++, mach_vm_allocate(TASK, &fixed, size, 0), KERN_SUCCESS);
    expect(step++, fixed, address);
    bytes = (unsigned char *)fixed;
    bytes[0] = 7;
    bytes[size - 1] = 9;
    expect(step++, bytes[0] == 7 && bytes[size - 1] == 9, 1);
    expect(step++, mach_vm_deallocate(TASK, fixed, size), KERN_SUCCESS);

    if (failures != 0) {
        puts("mach_vm:allocation contract failed");
        return 1;
    }
    puts("mach_vm:allocate, write, protect, fixed and errors passed");
    return 0;
}
