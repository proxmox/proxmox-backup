//! Helpers for common statistics tasks
use num_traits::cast::ToPrimitive;
use num_traits::NumAssignRef;

/// Calculates the sum of a list of numbers
/// ```
/// # use proxmox_backup::tools::statistics::sum;
/// # use num_traits::cast::ToPrimitive;
///
/// assert_eq!(sum(&[0,1,2,3,4,5]), 15);
/// assert_eq!(sum(&[-1,1,-2,2]), 0);
/// assert!((sum(&[0.0, 0.1,0.2]).to_f64().unwrap() - 0.3).abs() < 0.001);
/// assert!((sum(&[0.0, -0.1,0.2]).to_f64().unwrap() - 0.1).abs() < 0.001);
/// ```
pub fn sum<T>(list: &[T]) -> T
where
    T: NumAssignRef + ToPrimitive,
{
    let mut sum = T::zero();
    for num in list {
        sum += num;
    }
    sum
}

/// Calculates the mean of a variable x
/// ```
/// # use proxmox_backup::tools::statistics::mean;
///
/// assert!((mean(&[0,1,2,3,4,5]).unwrap() - 2.5).abs() < 0.001);
/// assert_eq!(mean::<u64>(&[]), None)
/// ```
pub fn mean<T>(list: &[T]) -> Option<f64>
where
    T: NumAssignRef + ToPrimitive,
{
    let len = list.len();
    if len == 0 {
        return None;
    }
    Some(sum(list).to_f64()? / (list.len() as f64))
}

/// Calculates the variance of a variable x
/// ```
/// # use proxmox_backup::tools::statistics::variance;
///
/// assert!((variance(&[1,2,3,4]).unwrap() - 1.25).abs() < 0.001);
/// assert_eq!(variance::<u64>(&[]), None)
/// ```
pub fn variance<T>(list: &[T]) -> Option<f64>
where
    T: NumAssignRef + ToPrimitive,
{
    covariance(list, list)
}

/// Calculates the (non-corrected) covariance of two variables x,y
pub fn covariance<X, Y>(x: &[X], y: &[Y]) -> Option<f64>
where
    X: NumAssignRef + ToPrimitive,
    Y: NumAssignRef + ToPrimitive,
{
    let len_x = x.len();
    let len_y = y.len();
    if len_x == 0 || len_y == 0 || len_x != len_y {
        return None;
    }

    let mean_x = mean(x)?;
    let mean_y = mean(y)?;

    let covariance: f64 = (0..len_x)
        .map(|i| {
            let x = x[i].to_f64().unwrap_or(0.0);
            let y = y[i].to_f64().unwrap_or(0.0);
            (x - mean_x) * (y - mean_y)
        })
        .sum();

    Some(covariance / (len_x as f64))
}

/// Returns the factors `(a,b)` of a linear regression `y = a + bx`
/// for the variables `[x,y]` or `None` if the lists are not the same length
/// ```
/// # use proxmox_backup::tools::statistics::linear_regression;
///
/// let x = &[0,1,2,3,4];
/// let y = &[-4,-2,0,2,4];
/// let (a,b) = linear_regression(x,y).unwrap();
/// assert!((a - -4.0).abs() < 0.001);
/// assert!((b - 2.0).abs() < 0.001);
/// ```
pub fn linear_regression<X, Y>(x: &[X], y: &[Y]) -> Option<(f64, f64)>
where
    X: NumAssignRef + ToPrimitive,
    Y: NumAssignRef + ToPrimitive,
{
    let len_x = x.len();
    let len_y = y.len();
    if len_x == 0 || len_y == 0 || len_x != len_y {
        return None;
    }

    let mean_x = mean(x)?;
    let mean_y = mean(y)?;

    let mut covariance = 0.0;
    let mut variance = 0.0;

    for i in 0..len_x {
        let x = x[i].to_f64()?;
        let y = y[i].to_f64()?;

        let x_mean_x = x - mean_x;

        covariance += x_mean_x * (y - mean_y);
        variance += x_mean_x * x_mean_x;
    }

    let beta = covariance / variance;
    let alpha = mean_y - beta * mean_x;
    Some((alpha, beta))
}
