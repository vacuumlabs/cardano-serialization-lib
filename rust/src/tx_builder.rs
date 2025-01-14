use super::*;
use super::fees;
use super::utils;
use std::collections::BTreeSet;

// comes from witsVKeyNeeded in the Ledger spec
fn witness_keys_for_cert(cert_enum: &Certificate, keys: &mut BTreeSet<Ed25519KeyHash>) {
    match &cert_enum.0 {
        // stake key registrations do not require a witness
        CertificateEnum::StakeRegistration(_cert) => {},
        CertificateEnum::StakeDeregistration(cert) => {
            keys.insert(cert.stake_credential().to_keyhash().unwrap());
        },
        CertificateEnum::StakeDelegation(cert) => {
            keys.insert(cert.stake_credential().to_keyhash().unwrap());
        },
        CertificateEnum::PoolRegistration(cert) => {
            for owner in &cert.pool_params().pool_owners().0 {
                keys.insert(owner.clone());
            }
            keys.insert(
                Ed25519KeyHash::from_bytes(cert.pool_params().operator().to_bytes()).unwrap()
            );
        },
        CertificateEnum::PoolRetirement(cert) => {
            keys.insert(
                Ed25519KeyHash::from_bytes(cert.pool_keyhash().to_bytes()).unwrap()
            );
        },
        CertificateEnum::GenesisKeyDelegation(cert) => {
            keys.insert(
                Ed25519KeyHash::from_bytes(cert.genesis_delegate_hash().to_bytes()).unwrap()
            );
        },
        // not witness as there is no single core node or genesis key that posts the certificate
        CertificateEnum::MoveInstantaneousRewardsCert(_cert) => {},
    }
}

fn min_fee(tx_builder: &TransactionBuilder) -> Result<Coin, JsError> {
    let body = tx_builder.build()?;

    let fake_key_root = Bip32PrivateKey::from_bip39_entropy(
        // art forum devote street sure rather head chuckle guard poverty release quote oak craft enemy
        &[0x0c, 0xcb, 0x74, 0xf3, 0x6b, 0x7d, 0xa1, 0x64, 0x9a, 0x81, 0x44, 0x67, 0x55, 0x22, 0xd4, 0xd8, 0x09, 0x7c, 0x64, 0x12],
        &[]
    );

    // recall: this includes keys for input, certs and withdrawals
    let vkeys = match tx_builder.input_types.vkeys.len() {
        0 => None,
        x => {
            let mut result = Vkeywitnesses::new();
            let raw_key = fake_key_root.to_raw_key();
            for _i in 0..x {
                result.add(&Vkeywitness::new(
                    &Vkey::new(&raw_key.to_public()),
                    &raw_key.sign([1u8; 100].as_ref())
                ));
            }
            Some(result)
        },
    };
    let script_keys = match tx_builder.input_types.scripts.len() {
        0 => None,
        _x => {
            // TODO: figure out how to populate fake witnesses for these
            return Err(JsError::from_str("Script inputs not supported yet"))
        },
    };
    let bootstrap_keys = match tx_builder.input_types.bootstraps.len() {
        0 => None,
        _x => {
            let mut result = BootstrapWitnesses::new();
            for addr in &tx_builder.input_types.bootstraps {
                // picking icarus over daedalus for fake witness generation shouldn't matter
                result.add(&make_icarus_bootstrap_witness(
                    &hash_transaction(&body),
                    &ByronAddress::from_bytes(addr.clone()).unwrap(),
                    &fake_key_root
                ));
            }
            Some(result)
        },
    };
    let witness_set = TransactionWitnessSet {
        vkeys: vkeys,
        native_scripts: script_keys,
        bootstraps: bootstrap_keys,
        // TODO: plutus support?
        plutus_scripts: None,
        plutus_data: None,
        redeemers: None,
    };
    let full_tx = Transaction {
        body,
        witness_set,
        auxiliary_data: tx_builder.auxiliary_data.clone(),
    };
    fees::min_fee(&full_tx, &tx_builder.fee_algo)
}


// We need to know how many of each type of witness will be in the transaction so we can calculate the tx fee
#[derive(Clone, Debug)]
struct MockWitnessSet {
    vkeys: BTreeSet<Ed25519KeyHash>,
    scripts: BTreeSet<ScriptHash>,
    bootstraps: BTreeSet<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct TxBuilderInput {
    input: TransactionInput,
    amount: Value, // we need to keep track of the amount in the inputs for input selection
}

#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct TransactionBuilder {
    minimum_utxo_val: BigNum,
    pool_deposit: BigNum,
    key_deposit: BigNum,
    fee_algo: fees::LinearFee,
    inputs: Vec<TxBuilderInput>,
    outputs: TransactionOutputs,
    fee: Option<Coin>,
    ttl: Option<Slot>, // absolute slot number
    certs: Option<Certificates>,
    withdrawals: Option<Withdrawals>,
    auxiliary_data: Option<AuxiliaryData>,
    validity_start_interval: Option<Slot>,
    input_types: MockWitnessSet,
    mint: Option<Mint>,
}

#[wasm_bindgen]
impl TransactionBuilder {
    // We have to know what kind of inputs these are to know what kind of mock witnesses to create since
    // 1) mock witnesses have different lengths depending on the type which changes the expecting fee
    // 2) Witnesses are a set so we need to get rid of duplicates to avoid over-estimating the fee
    pub fn add_key_input(&mut self, hash: &Ed25519KeyHash, input: &TransactionInput, amount: &Value) {
        self.inputs.push(TxBuilderInput {
            input: input.clone(),
            amount: amount.clone(),
        });
        self.input_types.vkeys.insert(hash.clone());
    }
    pub fn add_script_input(&mut self, hash: &ScriptHash, input: &TransactionInput, amount: &Value) {
        self.inputs.push(TxBuilderInput {
            input: input.clone(),
            amount: amount.clone(),
        });
        self.input_types.scripts.insert(hash.clone());
    }
    pub fn add_bootstrap_input(&mut self, hash: &ByronAddress, input: &TransactionInput, amount: &Value) {
        self.inputs.push(TxBuilderInput {
            input: input.clone(),
            amount: amount.clone(),
        });
        self.input_types.bootstraps.insert(hash.to_bytes());
    }
    
    pub fn add_input(&mut self, address: &Address, input: &TransactionInput, amount: &Value) {
        match &BaseAddress::from_address(address) {
            Some(addr) => {
                match &addr.payment_cred().to_keyhash() {
                    Some(hash) => return self.add_key_input(hash, input, amount),
                    None => (),
                }
                match &addr.payment_cred().to_scripthash() {
                    Some(hash) => return self.add_script_input(hash, input, amount),
                    None => (),
                }
            },
            None => (),
        }
        match &EnterpriseAddress::from_address(address) {
            Some(addr) => {
                match &addr.payment_cred().to_keyhash() {
                    Some(hash) => return self.add_key_input(hash, input, amount),
                    None => (),
                }
                match &addr.payment_cred().to_scripthash() {
                    Some(hash) => return self.add_script_input(hash, input, amount),
                    None => (),
                }
            },
            None => (),
        }
        match &PointerAddress::from_address(address) {
            Some(addr) => {
                match &addr.payment_cred().to_keyhash() {
                    Some(hash) => return self.add_key_input(hash, input, amount),
                    None => (),
                }
                match &addr.payment_cred().to_scripthash() {
                    Some(hash) => return self.add_script_input(hash, input, amount),
                    None => (),
                }
            },
            None => (),
        }
        match &ByronAddress::from_address(address) {
            Some(addr) => {
                return self.add_bootstrap_input(addr, input, amount);
            },
            None => (),
        }
    }

    /// calculates how much the fee would increase if you added a given output
    pub fn fee_for_input(&mut self, address: &Address, input: &TransactionInput, amount: &Value) -> Result<Coin, JsError> {
        let mut self_copy = self.clone();

        // we need some value for these for it to be a a valid transaction
        // but since we're only calculating the different between the fee of two transactions
        // it doesn't matter what these are set as, since it cancels out
        self_copy.set_fee(&to_bignum(0));

        let fee_before = min_fee(&self_copy)?;

        self_copy.add_input(&address, &input, &amount);
        let fee_after = min_fee(&self_copy)?;
        fee_after.checked_sub(&fee_before)
    }

    pub fn add_output(&mut self, output: &TransactionOutput) -> Result<(), JsError> {
        let min_ada = min_ada_required(&output.amount(), &self.minimum_utxo_val);
        if output.amount().coin() < min_ada {
            Err(JsError::from_str(&format!(
                "Value {} less than the minimum UTXO value {}",
                from_bignum(&output.amount().coin()),
                from_bignum(&min_ada)
            )))
        } else {
            self.outputs.add(output);
            Ok(())
        }
    }

    /// calculates how much the fee would increase if you added a given output
    pub fn fee_for_output(&mut self, output: &TransactionOutput) -> Result<Coin, JsError> {
        let mut self_copy = self.clone();

        // we need some value for these for it to be a a valid transaction
        // but since we're only calculating the different between the fee of two transactions
        // it doesn't matter what these are set as, since it cancels out
        self_copy.set_fee(&to_bignum(0));

        let fee_before = min_fee(&self_copy)?;

        self_copy.add_output(&output)?;
        let fee_after = min_fee(&self_copy)?;
        fee_after.checked_sub(&fee_before)
    }

    pub fn set_fee(&mut self, fee: &Coin) {
        self.fee = Some(fee.clone())
    }

    pub fn set_ttl(&mut self, ttl: Slot) {
        self.ttl = Some(ttl)
    }

    pub fn set_validity_start_interval(&mut self, validity_start_interval: Slot) {
        self.validity_start_interval = Some(validity_start_interval)
    }

    pub fn set_certs(&mut self, certs: &Certificates) {
        self.certs = Some(certs.clone());
        for cert in &certs.0 {
            witness_keys_for_cert(cert, &mut self.input_types.vkeys);
        };
    }

    pub fn set_withdrawals(&mut self, withdrawals: &Withdrawals) {
        self.withdrawals = Some(withdrawals.clone());
        for (withdrawal, _coin) in &withdrawals.0 {
            self.input_types.vkeys.insert(withdrawal.payment_cred().to_keyhash().unwrap().clone());
        };
    }

    pub fn set_auxiliary_data(&mut self, auxiliary_data: &AuxiliaryData) {
        self.auxiliary_data = Some(auxiliary_data.clone())
    }

    pub fn new(
        linear_fee: &fees::LinearFee,
        // protocol parameter that defines the minimum value a newly created UTXO can contain
        minimum_utxo_val: &Coin,
        pool_deposit: &BigNum, // protocol parameter
        key_deposit: &BigNum,  // protocol parameter
    ) -> Self {
        Self {
            minimum_utxo_val: minimum_utxo_val.clone(),
            key_deposit: key_deposit.clone(),
            pool_deposit: pool_deposit.clone(),
            fee_algo: linear_fee.clone(),
            inputs: Vec::new(),
            outputs: TransactionOutputs::new(),
            fee: None,
            ttl: None,
            certs: None,
            withdrawals: None,
            auxiliary_data: None,
            input_types: MockWitnessSet {
                vkeys: BTreeSet::new(),
                scripts: BTreeSet::new(),
                bootstraps: BTreeSet::new(),
            },
            validity_start_interval: None,
            mint: None
        }
    }

    /// does not include refunds or withdrawals
    pub fn get_explicit_input(&self) -> Result<Value, JsError> {
        self.inputs
            .iter()
            .try_fold(Value::new(&to_bignum(0)), |acc, ref tx_builder_input| {
                acc.checked_add(&tx_builder_input.amount)
            })
    }
    /// withdrawals and refunds
    pub fn get_implicit_input(&self) -> Result<Value, JsError> {
        internal_get_implicit_input(
            &self.withdrawals,
            &self.certs,
            &self.pool_deposit,
            &self.key_deposit,
        )
    }

    /// does not include fee
    pub fn get_explicit_output(&self) -> Result<Value, JsError> {
        self.outputs
            .0
            .iter()
            .try_fold(Value::new(&to_bignum(0)), |acc, ref output| {
                acc.checked_add(&output.amount())
            })
    }

    pub fn get_deposit(&self) -> Result<Coin, JsError> {
        internal_get_deposit(
            &self.certs,
            &self.pool_deposit,
            &self.key_deposit,
        )
    }

    pub fn get_fee_if_set(&self) -> Option<Coin> {
        self.fee.clone()
    }

    /// Warning: this function will mutate the /fee/ field
    pub fn add_change_if_needed(&mut self, address: &Address) -> Result<bool, JsError> {
        let fee = match &self.fee {
            None => self.min_fee(),
            // generating the change output involves changing the fee
            Some(_x) => {
                return Err(JsError::from_str(
                    "Cannot calculate change if fee was explicitly specified",
                ))
            }
        }?;

        let input_total = self
            .get_explicit_input()?
            .checked_add(&self.get_implicit_input()?)?;

        let output_total = self
            .get_explicit_output()?
            .checked_add(&Value::new(&self.get_deposit()?))?;

        use std::cmp::Ordering;
        match &input_total.partial_cmp(&output_total.checked_add(&Value::new(&fee))?) {
            Some(Ordering::Equal) => {
                // recall: min_fee assumed the fee was the maximum possible so we definitely have enough input to cover whatever fee it ends up being
                self.set_fee(&input_total.checked_sub(&output_total)?.coin());
                Ok(false)
            },
            Some(Ordering::Less) => Err(JsError::from_str("Insufficient input in transaction")),
            Some(Ordering::Greater) => {
                let change_estimator = input_total.checked_sub(&output_total)?;
                let min_ada = min_ada_required(&change_estimator.clone(), &self.minimum_utxo_val);

                let enough_input = |
                    builder: &mut TransactionBuilder,
                    burn_fee: &dyn Fn(&mut TransactionBuilder, &BigNum) -> Result<bool, JsError>,
                    add_change: &dyn Fn(&mut TransactionBuilder, &BigNum) -> Result<bool, JsError>
                | {
                    match change_estimator.coin() >= min_ada {
                        false => burn_fee(builder, &change_estimator.coin()),
                        true => {
                            // check how much the fee would increase if we added a change output
                            let fee_for_change = builder.fee_for_output(&TransactionOutput {
                                address: address.clone(),
                                amount: change_estimator.clone(),
                                // TODO: data hash?
                                data_hash: None,
                            })?;

                            let new_fee = fee.checked_add(&fee_for_change)?;
                            match change_estimator.coin() >= min_ada.checked_add(&Value::new(&new_fee).coin())? {
                                false => burn_fee(builder, &change_estimator.coin()),
                                true => add_change(builder, &new_fee)
                            }
                        }
                    }
                };

                let burn_extra = |
                    builder: &mut TransactionBuilder,
                    burn_amount: &BigNum
                | {
                    let has_assets = change_estimator.multiasset().map(|assets| assets.len() > 0).unwrap_or(false);
                    match has_assets {
                        false => {
                            // recall: min_fee assumed the fee was the maximum possible so we definitely have enough input to cover whatever fee it ends up being
                            builder.set_fee(burn_amount);
                            Ok(false) // not enough input to covert the extra fee from adding an output so we just burn whatever is left
                        },
                        true => Err(JsError::from_str("Not enough ADA leftover to include non-ADA assets in a change address")),
                    }
                };

                let add_change = |
                    builder: &mut TransactionBuilder,
                    new_fee: &BigNum
                | {
                    // recall: min_fee assumed the fee was the maximum possible so we definitely have enough input to cover whatever fee it ends up being
                    builder.set_fee(&new_fee);

                    builder.add_output(&TransactionOutput {
                        address: address.clone(),
                        amount: change_estimator.checked_sub(&Value::new(&new_fee.clone()))?,
                        data_hash: None, // TODO: How do we get DataHash?
                    })?;

                    Ok(true)
                };

                enough_input(
                    self,
                    &burn_extra,
                    &add_change
                )
            }
            None => Err(JsError::from_str("missing input for some native asset")),
        }
    }

    pub fn build(&self) -> Result<TransactionBody, JsError> {
        let fee = self.fee.ok_or_else(|| JsError::from_str("Fee not specified"))?;
        Ok(TransactionBody {
            inputs: TransactionInputs(self.inputs.iter().map(|ref tx_builder_input| tx_builder_input.input.clone()).collect()),
            outputs: self.outputs.clone(),
            fee: fee,
            ttl: self.ttl,
            certs: self.certs.clone(),
            withdrawals: self.withdrawals.clone(),
            update: None,
            auxiliary_data_hash: match &self.auxiliary_data {
                None => None,
                Some(x) => Some(utils::hash_auxiliary_data(x)),
            },
            validity_start_interval: self.validity_start_interval,
            mint: self.mint.clone(),
            // TODO: update for use with Alonzo
            script_data_hash: None,
            collateral: None,
            required_signers: None,
            network_id: None,
        })
    }

    /// warning: sum of all parts of a transaction must equal 0. You cannot just set the fee to the min value and forget about it
    /// warning: min_fee may be slightly larger than the actual minimum fee (ex: a few lovelaces)
    /// this is done to simplify the library code, but can be fixed later
    pub fn min_fee(&self) -> Result<Coin, JsError> {
        let mut self_copy = self.clone();
        self_copy.set_fee(&to_bignum(0x1_00_00_00_00));
        min_fee(&self_copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fees::*;

    fn genesis_id() -> TransactionHash {
        TransactionHash::from([0u8; TransactionHash::BYTE_COUNT])
    }

    fn root_key_15() -> Bip32PrivateKey {
        // art forum devote street sure rather head chuckle guard poverty release quote oak craft enemy
        let entropy = [0x0c, 0xcb, 0x74, 0xf3, 0x6b, 0x7d, 0xa1, 0x64, 0x9a, 0x81, 0x44, 0x67, 0x55, 0x22, 0xd4, 0xd8, 0x09, 0x7c, 0x64, 0x12];
        Bip32PrivateKey::from_bip39_entropy(&entropy, &[])
    }

    fn harden(index: u32) -> u32 {
        index | 0x80_00_00_00
    }

    #[test]
    fn build_tx_with_change() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(1), &to_bignum(1));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(NetworkInfo::testnet().network_id(), &spend_cred, &stake_cred).to_address();
        tx_builder.add_key_input(
            &spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(1_000_000))
        );
        tx_builder.add_output(&TransactionOutput::new(
            &addr_net_0,
            &Value::new(&to_bignum(10))
        )).unwrap();
        tx_builder.set_ttl(1000);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        );
        assert!(added_change.unwrap());
        assert_eq!(tx_builder.outputs.len(), 2);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder.get_explicit_output().unwrap().checked_add(&Value::new(&tx_builder.get_fee_if_set().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_without_change() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(1), &to_bignum(1));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(NetworkInfo::testnet().network_id(), &spend_cred, &stake_cred).to_address();
        tx_builder.add_key_input(
            &spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(1_000_000))
        );
        tx_builder.add_output(&TransactionOutput::new(
            &addr_net_0,
            &Value::new(&to_bignum(880_000))
        )).unwrap();
        tx_builder.set_ttl(1000);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        );
        assert!(!added_change.unwrap());
        assert_eq!(tx_builder.outputs.len(), 1);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder.get_explicit_output().unwrap().checked_add(&Value::new(&tx_builder.get_fee_if_set().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_with_certs() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder = TransactionBuilder::new(
            &linear_fee,
            &to_bignum(1),
            &to_bignum(1),
            &to_bignum(1_000_000),
        );
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        tx_builder.add_key_input(
            &spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(5_000_000))
        );
        tx_builder.set_ttl(1000);

        let mut certs = Certificates::new();
        certs.add(&Certificate::new_stake_registration(&StakeRegistration::new(&stake_cred)));
        certs.add(&Certificate::new_stake_delegation(&StakeDelegation::new(
            &stake_cred,
            &stake.to_raw_key().hash(), // in reality, this should be the pool owner's key, not ours
        )));
        tx_builder.set_certs(&certs);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        tx_builder.add_change_if_needed(
            &change_addr
        ).unwrap();
        assert_eq!(tx_builder.min_fee().unwrap().to_str(), "213502");
        assert_eq!(tx_builder.get_fee_if_set().unwrap().to_str(), "213502");
        assert_eq!(tx_builder.get_deposit().unwrap().to_str(), "1000000");
        assert_eq!(tx_builder.outputs.len(), 1);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder
                .get_explicit_output().unwrap()
                .checked_add(&Value::new(&tx_builder.get_fee_if_set().unwrap())).unwrap()
                .checked_add(&Value::new(&tx_builder.get_deposit().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_exact_amount() {
        // transactions where sum(input) == sum(output) exact should pass
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(0));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(0), &to_bignum(0));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();
        tx_builder.add_key_input(
            &&spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(5))
        );
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(NetworkInfo::testnet().network_id(), &spend_cred, &stake_cred).to_address();
        tx_builder.add_output(&TransactionOutput::new(
            &addr_net_0,
            &Value::new(&to_bignum(5))
        )).unwrap();
        tx_builder.set_ttl(0);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        ).unwrap();
        assert_eq!(added_change, false);
        let final_tx = tx_builder.build().unwrap();
        assert_eq!(final_tx.outputs().len(), 1);
    }

    #[test]
    fn build_tx_exact_change() {
        // transactions where we have exactly enough ADA to add change should pass
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(0));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(0), &to_bignum(0));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();
        tx_builder.add_key_input(
            &&spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(6))
        );
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &spend_cred,
            &stake_cred,
        )
        .to_address();
        tx_builder
            .add_output(&TransactionOutput::new(
                &addr_net_0,
                &Value::new(&to_bignum(5)),
            ))
            .unwrap();
        tx_builder.set_ttl(0);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        ).unwrap();
        assert_eq!(added_change, true);
        let final_tx = tx_builder.build().unwrap();
        assert_eq!(final_tx.outputs().len(), 2);
        assert_eq!(final_tx.outputs().get(1).amount().coin().to_str(), "1");
    }

    #[test]
    #[should_panic]
    fn build_tx_insufficient_deposit() {
        // transactions should fail with insufficient fees if a deposit is required
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(0));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(0), &to_bignum(5));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();
        tx_builder.add_key_input(
            &&spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(5)),
        );
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &spend_cred,
            &stake_cred,
        )
        .to_address();
        tx_builder
            .add_output(&TransactionOutput::new(
                &addr_net_0,
                &Value::new(&to_bignum(5)),
            ))
            .unwrap();
        tx_builder.set_ttl(0);

        // add a cert which requires a deposit
        let mut certs = Certificates::new();
        certs.add(&Certificate::new_stake_registration(
            &StakeRegistration::new(&stake_cred),
        ));
        tx_builder.set_certs(&certs);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &change_cred,
            &stake_cred,
        )
        .to_address();

        tx_builder.add_change_if_needed(&change_addr).unwrap();
    }

    #[test]
    fn build_tx_with_inputs() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(1), &to_bignum(1));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());

        {
            assert_eq!(tx_builder.fee_for_input(
                &EnterpriseAddress::new(
                    NetworkInfo::testnet().network_id(),
                    &spend_cred
                ).to_address(),
                &TransactionInput::new(&genesis_id(), 0),
                &Value::new(&to_bignum(1_000_000))
            ).unwrap().to_str(), "69500");
            tx_builder.add_input(
                &EnterpriseAddress::new(
                    NetworkInfo::testnet().network_id(),
                    &spend_cred
                ).to_address(),
                &TransactionInput::new(&genesis_id(), 0),
                &Value::new(&to_bignum(1_000_000))
            );
        }
        tx_builder.add_input(
            &BaseAddress::new(
                NetworkInfo::testnet().network_id(),
                &spend_cred,
                &stake_cred
            ).to_address(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(1_000_000))
        );
        tx_builder.add_input(
            &PointerAddress::new(
                NetworkInfo::testnet().network_id(),
                &spend_cred,
                &Pointer::new(
                    0,
                    0,
                    0
                )
            ).to_address(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(1_000_000))
        );
        tx_builder.add_input(
            &ByronAddress::icarus_from_key(
                &spend, NetworkInfo::testnet().protocol_magic()
            ).to_address(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(&to_bignum(1_000_000))
        );

        assert_eq!(tx_builder.inputs.len(), 4);
    }

    #[test]
    fn build_tx_with_native_assets_change() {
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(1));
        let minimum_utxo_value = to_bignum(1);
        let mut tx_builder = TransactionBuilder::new(
            &linear_fee,
            &minimum_utxo_value,
            &to_bignum(0),
            &to_bignum(0),
        );
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let policy_id = &PolicyID::from([0u8; 28]);
        let name = AssetName::new(vec![0u8, 1, 2, 3]).unwrap();

        let ma_input1 = 100;
        let ma_input2 = 200;
        let ma_output1 = 60;

        let multiassets = [ma_input1, ma_input2, ma_output1]
            .iter()
            .map(|input| {
                let mut multiasset = MultiAsset::new();
                multiasset.insert(policy_id, &{
                    let mut assets = Assets::new();
                    assets.insert(&name, &to_bignum(*input));
                    assets
                });
                multiasset
            })
            .collect::<Vec<MultiAsset>>();

        for (multiasset, ada) in multiassets
            .iter()
            .zip([10u64, 10].iter().cloned().map(to_bignum))
        {
            let mut input_amount = Value::new(&ada);
            input_amount.set_multiasset(multiasset);

            tx_builder.add_key_input(
                &&spend.to_raw_key().hash(),
                &TransactionInput::new(&genesis_id(), 0),
                &input_amount,
            );
        }

        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());

        let addr_net_0 = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &spend_cred,
            &stake_cred,
        )
        .to_address();

        let mut output_amount = Value::new(&to_bignum(1));
        output_amount.set_multiasset(&multiassets[2]);

        tx_builder
            .add_output(&TransactionOutput::new(&addr_net_0, &output_amount))
            .unwrap();

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &change_cred,
            &stake_cred,
        )
        .to_address();

        let added_change = tx_builder.add_change_if_needed(&change_addr).unwrap();
        assert_eq!(added_change, true);
        let final_tx = tx_builder.build().unwrap();
        assert_eq!(final_tx.outputs().len(), 2);
        assert_eq!(
            final_tx.outputs().get(0).amount().coin(),
            minimum_utxo_value
        );
        assert_eq!(
            final_tx
                .outputs()
                .get(1)
                .amount()
                .multiasset()
                .unwrap()
                .get(policy_id)
                .unwrap()
                .get(&name)
                .unwrap(),
            to_bignum(ma_input1 + ma_input2 - ma_output1)
        );
    }

    #[test]
    #[should_panic]
    fn build_tx_leftover_assets() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(1), &to_bignum(1));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(NetworkInfo::testnet().network_id(), &spend_cred, &stake_cred).to_address();

        // add an input that contains an asset not present in the output
        let policy_id = &PolicyID::from([0u8; 28]);
        let name = AssetName::new(vec![0u8, 1, 2, 3]).unwrap();
        let mut input_amount = Value::new(&to_bignum(1_000_000));
        let mut input_multiasset = MultiAsset::new();
        input_multiasset.insert(policy_id, &{
            let mut assets = Assets::new();
            assets.insert(&name, &to_bignum(100));
            assets
        });
        input_amount.set_multiasset(&input_multiasset);
        tx_builder.add_key_input(
            &spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &input_amount
        );

        tx_builder.add_output(&TransactionOutput::new(
            &addr_net_0,
            &Value::new(&to_bignum(880_000))
        )).unwrap();
        tx_builder.set_ttl(1000);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        );
        assert!(!added_change.unwrap());
        assert_eq!(tx_builder.outputs.len(), 1);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder.get_explicit_output().unwrap().checked_add(&Value::new(&tx_builder.get_fee_if_set().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_burn_less_than_min_ada() {
        let linear_fee = LinearFee::new(&to_bignum(44), &to_bignum(155381));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1000000), &to_bignum(500000000), &to_bignum(2000000));

        let output_addr = ByronAddress::from_base58("Ae2tdPwUPEZD9QQf2ZrcYV34pYJwxK4vqXaF8EXkup1eYH73zUScHReM42b").unwrap();
        tx_builder.add_output(&TransactionOutput::new(
            &output_addr.to_address(),
            &Value::new(&to_bignum(2_000_000))
        )).unwrap();

        tx_builder.add_input(
            &ByronAddress::from_base58("Ae2tdPwUPEZ5uzkzh1o2DHECiUi3iugvnnKHRisPgRRP3CTF4KCMvy54Xd3").unwrap().to_address(),
            &TransactionInput::new(
                &genesis_id(),
                0
            ),
            &Value::new(&to_bignum(2_400_000))
        );
        
        tx_builder.set_ttl(1);

        let change_addr = ByronAddress::from_base58("Ae2tdPwUPEZGUEsuMAhvDcy94LKsZxDjCbgaiBBMgYpR8sKf96xJmit7Eho").unwrap();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr.to_address()
        );
        assert!(!added_change.unwrap());
        assert_eq!(tx_builder.outputs.len(), 1);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder.get_explicit_output().unwrap().checked_add(&Value::new(&tx_builder.get_fee_if_set().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_burn_empty_assets() {
        let linear_fee = LinearFee::new(&to_bignum(44), &to_bignum(155381));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1000000), &to_bignum(500000000), &to_bignum(2000000));

        let output_addr = ByronAddress::from_base58("Ae2tdPwUPEZD9QQf2ZrcYV34pYJwxK4vqXaF8EXkup1eYH73zUScHReM42b").unwrap();
        tx_builder.add_output(&TransactionOutput::new(
            &output_addr.to_address(),
            &Value::new(&to_bignum(2_000_000))
        )).unwrap();

        let mut input_value = Value::new(&to_bignum(2_400_000));
        input_value.set_multiasset(&MultiAsset::new());
        tx_builder.add_input(
            &ByronAddress::from_base58("Ae2tdPwUPEZ5uzkzh1o2DHECiUi3iugvnnKHRisPgRRP3CTF4KCMvy54Xd3").unwrap().to_address(),
            &TransactionInput::new(
                &genesis_id(),
                0
            ),
            &input_value
        );
        
        tx_builder.set_ttl(1);

        let change_addr = ByronAddress::from_base58("Ae2tdPwUPEZGUEsuMAhvDcy94LKsZxDjCbgaiBBMgYpR8sKf96xJmit7Eho").unwrap();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr.to_address()
        );
        assert!(!added_change.unwrap());
        assert_eq!(tx_builder.outputs.len(), 1);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap().coin(),
            tx_builder.get_explicit_output().unwrap().checked_add(&Value::new(&tx_builder.get_fee_if_set().unwrap())).unwrap().coin()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }
}
