__thread int library_count=17;
__thread char library_aligned[128] __attribute__((aligned(128)));
int library_step(void) {
    if ((unsigned long)library_aligned&127) return -1000;
    library_aligned[127]=3;return ++library_count;
}
