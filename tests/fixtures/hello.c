typedef unsigned long size_t;
extern int puts(const char *);
extern size_t strlen(const char *);
extern void *malloc(size_t);
extern void free(void *);
extern void *memcpy(void *, const void *, size_t);
extern int memcmp(const void *, const void *, size_t);
static const char *message = "Adapter: Mach-O code executed on Windows";
static int initialized;
__attribute__((constructor)) static void initialize(void) { initialized=73; }
int main(int argc, char **argv) {
    if (initialized != 73 || argc != 2 || strlen(argv[1]) != 5) return 91;
    char *copy=malloc(6);
    if (!copy) return 92;
    memcpy(copy,argv[1],6);
    int good=memcmp(copy,"hello",6)==0;
    free(copy);
    if (!good) return 93;
    puts(message);
    return 0;
}
