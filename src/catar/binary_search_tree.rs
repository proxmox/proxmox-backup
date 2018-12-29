/// Helpers to generate a binary search tree stored in an array from a
/// sorted array.
///
/// Specifically, for any given sorted * array 'input' permute the
/// array so that the following rule holds:
///
/// For each array item with index i, the item at 2*i+1 is smaller and
/// the item 2*i+2 is larger.
///
/// This structure permits efficient (meaning: O(log(n)) binary
/// searches: start with item i=0 (i.e. the root of the BST), compare
/// the value with the searched item, if smaller proceed at item
/// i*2+1, if larger proceed at item i*2+2, and repeat, until either
/// the item is found, or the indexes grow beyond the array size,
/// which means the entry does not exist.
///
/// Effectively this implements bisection, but instead of jumping
/// around wildly in the array during a single search we only search
/// with strictly monotonically increasing indexes.
///
/// Algorithm is from casync (camakebst.c), simplified and optimized
/// for rust. Permutation function originally by L. Bressel, 2017.
///
///

fn copy_binary_search_tree_inner<F:  FnMut(usize, usize)>(
    copy_func: &mut F,
    n: usize,
    o: usize, // Note: virtual offset for input array
    e: usize,
    i: usize,
) {
    let p = 1 << e;

    let t = p + (p>>1) - 1;

    let m = if n > t {
        // |...........p.............t....n........(2p)|
        p - 1
    } else {
        // |...........p.....n.......t.............(2p)|
        p - 1 - (t-n)
    };

    (copy_func)(o+m, i);

    if m > 0 {
        copy_binary_search_tree_inner(copy_func, m, 0, e-1, i*2+1);
    }

    if (m + 1) < n {
        copy_binary_search_tree_inner(copy_func, n-m-1, o+m+1, e-1, i*2+2);
    }
}

pub fn copy_binary_search_tree<F:  FnMut(usize, usize)>(
    n: usize,
    mut copy_func: F,
) {
    if n == 0 { return };
    let e = (64 - n.leading_zeros() - 1) as usize; // fast log2(n)
    copy_binary_search_tree_inner(&mut copy_func, n, 0, e, 0);
}
