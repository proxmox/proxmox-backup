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
/// for rust. Permutation function originally by L. Bressel, 2017. We
/// pass permutation info to user provided callback, which actually
/// implements the data copy.
///

// NOTES:
//
// https://en.wikipedia.org/wiki/Binary_heap
// https://en.wikipedia.org/wiki/Heapsort
//
// ==> Maype it is possible to build a sorted array from unsorted
// array inplace, using heapsort?

fn copy_binary_search_tree_inner<F:  FnMut(usize, usize)>(
    copy_func: &mut F,
    // we work on input array input[o..o+n]
    n: usize,
    o: usize,
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
        copy_binary_search_tree_inner(copy_func, m, o, e-1, i*2+1);
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


#[test]
fn test_binary_search_tree() {

    fn run_test(len: usize) -> Vec<usize> {

        const MARKER: usize = 0xfffffff;
        let mut output = vec![];
        for i in 0..len { output.push(MARKER); }
        copy_binary_search_tree(len, |s, d| {
            assert!(output[d] == MARKER);
            output[d] = s;
        });
        if len < 32 { println!("GOT:{}:{:?}", len, output); }
        for i in 0..len {
            assert!(output[i] != MARKER);
        }
        output
    }

    assert!(run_test(0).len() == 0);
    assert!(run_test(1) == [0]);
    assert!(run_test(2) == [1,0]);
    assert!(run_test(3) == [1,0,2]);
    assert!(run_test(4) == [2,1,3,0]);
    assert!(run_test(5) == [3,1,4,0,2]);
    assert!(run_test(6) == [3,1,5,0,2,4]);
    assert!(run_test(7) == [3,1,5,0,2,4,6]);
    assert!(run_test(8) == [4,2,6,1,3,5,7,0]);
    assert!(run_test(9) == [5,3,7,1,4,6,8,0,2]);
    assert!(run_test(10) == [6,3,8,1,5,7,9,0,2,4]);
    assert!(run_test(11) == [7,3,9,1,5,8,10,0,2,4,6]);
    assert!(run_test(12) == [7,3,10,1,5,9,11,0,2,4,6,8]);
    assert!(run_test(13) == [7,3,11,1,5,9,12,0,2,4,6,8,10]);
    assert!(run_test(14) == [7,3,11,1,5,9,13,0,2,4,6,8,10,12]);
    assert!(run_test(15) == [7,3,11,1,5,9,13,0,2,4,6,8,10,12,14]);
    assert!(run_test(16) == [8,4,12,2,6,10,14,1,3,5,7,9,11,13,15,0]);
    assert!(run_test(17) == [9,5,13,3,7,11,15,1,4,6,8,10,12,14,16,0,2]);

    for len in 18..1000 {
        run_test(len);
    }
}
