extern int puts(const char *);
extern int middle(void);
extern int leaf(void);
extern int shared_value;
extern int *selected;
static int ready;
__attribute__((constructor)) static void main_init(void) { ready=(middle()==22 && leaf()==20 && shared_value==20); puts("main:init"); }
__attribute__((destructor)) static void main_term(void) { puts("main:fini"); }
int main(void) { if (!ready || *selected!=7) return 95; puts("dylib:passed"); return 0; }
