extern int puts(const char *);
__attribute__((constructor)) static void should_not_run(void) {
    puts("objc:unexpected constructor");
}
__attribute__((objc_root_class))
@interface CategoryRoot
+ (int)value;
@end
@implementation CategoryRoot
+ (int)value {
    return 1;
}
@end
@interface CategoryRoot (Extra)
+ (int)extra;
@end
@implementation CategoryRoot (Extra)
+ (int)extra {
    return 2;
}
@end
int main(void) { return [CategoryRoot extra]; }
