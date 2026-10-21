extern int puts(const char *);
int shared_value=17;
int numbers[3]={3,7,11};
static int initialized;
__attribute__((constructor)) static void leaf_init(void) { initialized++; puts("leaf:init"); }
__attribute__((destructor)) static void leaf_term(void) { puts("leaf:fini"); }
int leaf(void) { return initialized==1 ? shared_value : -1000; }
