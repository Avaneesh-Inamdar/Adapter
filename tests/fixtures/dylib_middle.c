extern int puts(const char *);
extern int leaf(void);
extern int shared_value;
extern int numbers[3];
int *selected=&numbers[1];
static int ready;
__attribute__((constructor)) static void middle_init(void) { ready=(leaf()==17); shared_value+=3; puts("middle:init"); }
__attribute__((destructor)) static void middle_term(void) { puts("middle:fini"); }
int middle(void) { return ready ? leaf()+2 : -2000; }
