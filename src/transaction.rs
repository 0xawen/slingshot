use bulletproofs::r1cs::{Assignment, ConstraintSystem, Variable};
use curve25519_dalek::scalar::Scalar;
use gadgets::{merge, pad, range_proof, split, value_shuffle};
use std::cmp::max;
use subtle::{ConditionallySelectable, ConstantTimeEq};
use util::{SpacesuitError, Value};

// Enforces that the outputs are a valid rearrangement of the inputs, following the
// soundness and secrecy requirements in the spacesuit spec.
// TODO: add padding for different input and output sizes. (currently assuming n = m)
pub fn fill_cs<CS: ConstraintSystem>(
    cs: &mut CS,
    inputs: Vec<Value>,
    merge_in: Vec<Value>,
    merge_mid: Vec<Value>,
    merge_out: Vec<Value>,
    split_in: Vec<Value>,
    split_mid: Vec<Value>,
    split_out: Vec<Value>,
    outputs: Vec<Value>,
) -> Result<(), SpacesuitError> {
    let m = inputs.len();
    let n = outputs.len();
    let inner_merge_count = max(m as isize - 2, 0) as usize;
    let inner_split_count = max(n as isize - 2, 0) as usize;
    if inputs.len() != merge_in.len()
        || merge_in.len() != merge_out.len()
        || split_in.len() != split_out.len()
        || split_out.len() != outputs.len()
        || merge_mid.len() != inner_merge_count
        || split_mid.len() != inner_split_count
    {
        return Err(SpacesuitError::InvalidR1CSConstruction);
    }

    // Shuffle 1
    // Group the inputs by flavor.
    // Choice -> Ordering conversion? seems wrong...
    // shuffle1_outputs.sort_by(|cur, next| cur.a.1.ct_eq(&next.a.1));
    value_shuffle::fill_cs(cs, inputs, merge_in.clone())?;

    // Merge
    // Combine all the inputs of the same flavor. If different flavors, do not combine.
    // TODO: use merge_mid
    merge::fill_cs(cs, merge_in, merge_mid, merge_out.clone())?;

    // Shuffle 2
    value_shuffle::fill_cs(cs, merge_out, split_in.clone())?;

    // Split
    // Combine all the outputs of the same flavor. If different flavors, do not combine.
    // TODO: use split_mid
    split::fill_cs(cs, split_in, split_mid, split_out.clone())?;

    // Shuffle 3
    // Group the outputs by flavor.
    value_shuffle::fill_cs(cs, split_out, outputs.clone())?;

    // Range Proof
    for output in outputs {
        range_proof::fill_cs(cs, output.q, 64)?;
    }

    Ok(())
}

pub fn make_commitments(
    inputs: Vec<(u64, u64, u64)>,
    outputs: Vec<(u64, u64, u64)>,
) -> Vec<Scalar> {
    let m = inputs.len();
    let n = outputs.len();
    let inner_merge_count = max(m as isize - 2, 0) as usize;
    let inner_split_count = max(n as isize - 2, 0) as usize;
    let commitment_count = 2 * m + inner_merge_count + 2 * n + inner_split_count;
    let mut v = Vec::with_capacity(commitment_count);

    // Input to transaction
    for i in 0..m {
        v.push(Scalar::from(inputs[i].0));
        v.push(Scalar::from(inputs[i].1));
        v.push(Scalar::from(inputs[i].2));
    }
    // dummy logic here
    // Shuffle 1 output, input to merge
    for i in 0..m {
        v.push(Scalar::from(inputs[i].0));
        v.push(Scalar::from(inputs[i].1));
        v.push(Scalar::from(inputs[i].2));
    }
    // Intermediate merge
    for i in 0..inner_merge_count {
        v.push(Scalar::from(inputs[i + 1].0));
        v.push(Scalar::from(inputs[i + 1].1));
        v.push(Scalar::from(inputs[i + 1].2));
    }
    // Output to merge, input to shuffle 2
    for i in 0..m {
        v.push(Scalar::from(inputs[i].0));
        v.push(Scalar::from(inputs[i].1));
        v.push(Scalar::from(inputs[i].2));
    }
    // Output to shuffle 2, input to split
    for i in 0..n {
        v.push(Scalar::from(inputs[i].0));
        v.push(Scalar::from(inputs[i].1));
        v.push(Scalar::from(inputs[i].2));
    }
    // Intermediate split
    for i in 0..inner_split_count {
        v.push(Scalar::from(inputs[i + 1].0));
        v.push(Scalar::from(inputs[i + 1].1));
        v.push(Scalar::from(inputs[i + 1].2));
    }
    // Output to split, input to shuffle 3
    for i in 0..n {
        v.push(Scalar::from(inputs[i].0));
        v.push(Scalar::from(inputs[i].1));
        v.push(Scalar::from(inputs[i].2));
    }
    // dummy logic ends
    // Output of transaction
    for i in 0..n {
        v.push(Scalar::from(outputs[i].0));
        v.push(Scalar::from(outputs[i].1));
        v.push(Scalar::from(outputs[i].2));
    }

    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use bulletproofs::r1cs::{Assignment, ProverCS, VerifierCS};
    use bulletproofs::{BulletproofGens, PedersenGens};
    use merlin::Transcript;

    #[test]
    fn transaction() {
        // m=1, n=1
        assert!(transaction_helper(vec![(1, 2, 3)], vec![(1, 2, 3)]).is_ok());
        assert!(transaction_helper(vec![(4, 5, 6)], vec![(4, 5, 6)]).is_ok());
        assert!(transaction_helper(vec![(1, 2, 3)], vec![(4, 5, 6)]).is_err());

        // m=2, n=2, only shuffle (all different flavors)
        assert!(transaction_helper(vec![(1, 2, 3), (4, 5, 6)], vec![(1, 2, 3), (4, 5, 6)]).is_ok());
        assert!(transaction_helper(vec![(1, 2, 3), (4, 5, 6)], vec![(4, 5, 6), (1, 2, 3)]).is_ok());
        assert!(transaction_helper(vec![(4, 5, 6), (4, 5, 6)], vec![(4, 5, 6), (4, 5, 6)]).is_ok());
        assert!(
            transaction_helper(vec![(1, 2, 3), (1, 2, 3)], vec![(4, 5, 6), (1, 2, 3)]).is_err()
        );
        assert!(transaction_helper(vec![(1, 2, 3), (4, 5, 6)], vec![(1, 2, 3), (4, 5, 6)]).is_ok());

        // m=2, n=2, uses merge and split (has multiple inputs or outputs of same flavor)
        // $5 + $3 = $5 + $3
        assert!(transaction_helper(vec![(5, 9, 9), (3, 9, 9)], vec![(5, 9, 9), (3, 9, 9)]).is_ok());
        // $5 + $3 = $1 + $7
        assert!(transaction_helper(vec![(5, 9, 9), (3, 9, 9)], vec![(1, 9, 9), (7, 9, 9)]).is_ok());

        // m=3, n=3, only shuffle
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)]
            ).is_ok()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(1, 2, 3), (8, 9, 10), (4, 5, 6)]
            ).is_ok()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(4, 5, 6), (1, 2, 3), (8, 9, 10)]
            ).is_ok()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(4, 5, 6), (8, 9, 10), (1, 2, 3)]
            ).is_ok()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(8, 9, 10), (1, 2, 3), (4, 5, 6)]
            ).is_ok()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(8, 9, 10), (4, 5, 6), (1, 2, 3)]
            ).is_ok()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(10, 20, 30), (4, 5, 6), (8, 9, 10)]
            ).is_err()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(1, 2, 3), (40, 50, 60), (8, 9, 10)]
            ).is_err()
        );
        assert!(
            transaction_helper(
                vec![(1, 2, 3), (4, 5, 6), (8, 9, 10)],
                vec![(1, 2, 3), (4, 5, 6), (98, 99, 100)]
            ).is_err()
        );
    }

    fn transaction_helper(
        inputs: Vec<(u64, u64, u64)>,
        outputs: Vec<(u64, u64, u64)>,
    ) -> Result<(), SpacesuitError> {
        // Common
        let pc_gens = PedersenGens::default();
        let bp_gens = BulletproofGens::new(500, 1);
        let m = inputs.len();
        let n = outputs.len();

        // Prover's scope
        let (proof, commitments) = {
            // Prover makes a `ConstraintSystem` instance representing a transaction gadget
            // Make v vector
            let v = make_commitments(inputs, outputs);

            // Make v_blinding vector using RNG from transcript
            let mut prover_transcript = Transcript::new(b"TransactionTest");
            let mut rng = {
                let mut builder = prover_transcript.build_rng();

                // Commit the secret values
                for &v_i in &v {
                    builder = builder.commit_witness_bytes(b"v_i", v_i.as_bytes());
                }
                use rand::thread_rng;
                builder.finalize(&mut thread_rng())
            };
            let v_blinding: Vec<Scalar> = (0..v.len()).map(|_| Scalar::random(&mut rng)).collect();

            let (mut prover_cs, variables, commitments) = ProverCS::new(
                &bp_gens,
                &pc_gens,
                &mut prover_transcript,
                v.clone(),
                v_blinding,
            );

            // Prover adds constraints to the constraint system
            let v_assignments = v.iter().map(|v_i| Assignment::from(*v_i)).collect();
            let (inp, m_i, m_m, m_o, s_i, s_m, s_o, out) =
                value_helper(variables, v_assignments, m, n);

            fill_cs(&mut prover_cs, inp, m_i, m_m, m_o, s_i, s_m, s_o, out)?;
            let proof = prover_cs.prove()?;

            (proof, commitments)
        };

        // Verifier makes a `ConstraintSystem` instance representing a merge gadget
        let mut verifier_transcript = Transcript::new(b"TransactionTest");
        let (mut verifier_cs, variables) =
            VerifierCS::new(&bp_gens, &pc_gens, &mut verifier_transcript, commitments);

        // Verifier allocates variables and adds constraints to the constraint system
        let v_assignments = vec![Assignment::Missing(); variables.len()];
        let (inp, m_i, m_m, m_o, s_i, s_m, s_o, out) = value_helper(variables, v_assignments, m, n);

        assert!(fill_cs(&mut verifier_cs, inp, m_i, m_m, m_o, s_i, s_m, s_o, out).is_ok());

        Ok(verifier_cs.verify(&proof)?)
    }

    fn value_helper(
        variables: Vec<Variable>,
        assignments: Vec<Assignment>,
        m: usize,
        n: usize,
    ) -> (
        Vec<Value>,
        Vec<Value>,
        Vec<Value>,
        Vec<Value>,
        Vec<Value>,
        Vec<Value>,
        Vec<Value>,
        Vec<Value>,
    ) {
        let inner_merge_count = max(m as isize - 2, 0) as usize;
        let inner_split_count = max(n as isize - 2, 0) as usize;
        let val_count = variables.len() / 3;

        let mut values = Vec::with_capacity(val_count);
        for i in 0..val_count {
            values.push(Value {
                q: (variables[i * 3], assignments[i * 3]),
                a: (variables[i * 3 + 1], assignments[i * 3 + 1]),
                t: (variables[i * 3 + 2], assignments[i * 3 + 2]),
            });
        }

        // TODO: surely there's a better way to do this
        let mut index = 0;
        let inp = &values[index..index + m];
        index = index + m;
        let m_i = &values[index..index + m];
        index = index + m;
        let m_m = &values[index..index + inner_merge_count];
        index = index + inner_merge_count;
        let m_o = &values[index..index + m];
        index = index + m;
        let s_i = &values[index..index + n];
        index = index + n;
        let s_m = &values[index..index + inner_split_count];
        index = index + inner_split_count;
        let s_o = &values[index..index + n];
        index = index + n;
        let out = &values[index..index + n];

        (
            inp.to_vec(),
            m_i.to_vec(),
            m_m.to_vec(),
            m_o.to_vec(),
            s_i.to_vec(),
            s_m.to_vec(),
            s_o.to_vec(),
            out.to_vec(),
        )
    }
}
