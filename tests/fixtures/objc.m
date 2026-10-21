__attribute__((objc_root_class))
@interface Probe
+ (int)value;
@end
@implementation Probe
+ (int)value { return 42; }
@end
int main(void) { return [Probe value]; }
