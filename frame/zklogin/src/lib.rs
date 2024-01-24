#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(test)]
mod tests;

use scale_codec::{Codec, Encode};

use frame_support::dispatch::{
    DispatchClass, DispatchInfo, DispatchResultWithPostInfo, GetDispatchInfo, PostDispatchInfo,
};
use sp_runtime::{
    traits::{Applyable, BlockNumberProvider, Checkable, Dispatchable, Extrinsic, StaticLookup},
    transaction_validity::{InvalidTransaction, TransactionValidityError, UnknownTransaction},
};
use sp_std::prelude::*;
use zklogin_support::ReplaceSender;
use zp_zklogin::{verify_zk_login, ZkLoginInputs};

type AccountIdLookupOf<T> = <<T as frame_system::Config>::Lookup as StaticLookup>::Source;

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use frame_support::{dispatch::PostDispatchInfo, pallet_prelude::*};
    use frame_system::pallet_prelude::*;
    use sp_core::{crypto::AccountId32, U256};
    use zp_zklogin::{JwkId, ZkLoginEnv, EPH_PUB_KEY_LEN};

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>
            + TryInto<Event<Self>>;

        /// Same as `Executive`, required by `Checkable` for `Self::Extrinsic`
        type Context: Default;

        type Extrinsic: Extrinsic<Call = Self::RuntimeCall>
            + Checkable<Self::Context, Checked = Self::CheckedExtrinsic>
            + Codec
            + TypeInfo
            + Member;

        type CheckedExtrinsic: Applyable<Call = Self::RuntimeCall>
            + GetDispatchInfo
            + ReplaceSender<AccountId = Self::AccountId>;

        /// Same as `Executive`
        type UnsignedValidator: ValidateUnsigned<Call = Self::RuntimeCall>;

        type BlockNumberProvider: BlockNumberProvider<BlockNumber = BlockNumberFor<Self>>;
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        ZkLoginExecuted { result: DispatchResult },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// TODO
        EphKeyExpired,
        /// TODO
        InvalidTransaction,
        UnknownTransactionCannotLookup,
        UnknownTransactionNoUnsignedValidator,
        UnknownTransactionCustom,
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

    #[pallet::call]
    impl<T: Config> Pallet<T>
    where
        T::RuntimeCall: Dispatchable<Info = DispatchInfo, PostInfo = PostDispatchInfo>,
    {
        #[pallet::call_index(0)]
        #[pallet::weight(0)]
        pub fn submit_zklogin_unsigned(
            origin: OriginFor<T>,
            uxt: Box<T::Extrinsic>,
            address_seed: AccountIdLookupOf<T>,
            _inputs: ZkLoginInputs,
            _jwk_id: JwkId,
            expire_at: u32,
            _eph_pubkey: [u8; EPH_PUB_KEY_LEN],
        ) -> DispatchResultWithPostInfo {
            // make sure this call is unsigned signed
            ensure_none(origin)?;

            // check ehpkey's expiration time
            let now = T::BlockNumberProvider::current_block_number();
            let expire_at: BlockNumberFor<T> = expire_at.into();
            ensure!(expire_at <= now, Error::<T>::EphKeyExpired);

            // execute real call
            let r = Executive::<T>::apply_extrinsic(uxt, address_seed);
            let exec_res: DispatchResult = r.map(|_| ()).map_err(|e| e.error);
            Self::deposit_event(Event::ZkLoginExecuted { result: exec_res });
            r
        }
    }

    #[pallet::validate_unsigned]
    impl<T: Config> ValidateUnsigned for Pallet<T>
    where
        T::RuntimeCall: Dispatchable<Info = DispatchInfo, PostInfo = PostDispatchInfo>,
        T: frame_system::Config<AccountId = AccountId32>,
    {
        type Call = Call<T>;

        fn validate_unsigned(source: TransactionSource, call: &Self::Call) -> TransactionValidity {
            // TODO no need?
            match source {
                TransactionSource::InBlock | TransactionSource::External => { /* allowed */ }
                _ => return InvalidTransaction::Call.into(),
            };

            // verify signature
            match call {
                Call::submit_zklogin_unsigned {
                    uxt,
                    address_seed,
                    inputs,
                    jwk_id,
                    expire_at,
                    eph_pubkey,
                } => {
                    // only signed extrinsic is allowed
                    if !uxt.is_signed().unwrap_or(false) {
                        return InvalidTransaction::Call.into();
                    }
                    let address_seed = T::Lookup::lookup(address_seed.clone())?;

                    let encoded = uxt.encode();
                    let encoded_len = encoded.len();

                    let mut xt = uxt.clone().check(&T::Context::default())?;
                    xt.replace_sender(address_seed.clone());
                    // Decode parameters and dispatch
                    let dispatch_info = xt.get_dispatch_info();
                    // mandatory extrinsic is not allowed to use zklogin
                    if dispatch_info.class == DispatchClass::Mandatory {
                        return InvalidTransaction::BadMandatory.into();
                    }

                    // TODO remove this!
                    let address_seed = U256::from_big_endian(address_seed.as_ref());

                    // validate zk proof
                    verify_zk_login(
                        address_seed,
                        inputs,
                        jwk_id,
                        *expire_at,
                        eph_pubkey,
                        &ZkLoginEnv::Prod,
                    )
                    .map_err(|_| InvalidTransaction::BadProof)?;

                    xt.validate::<T::UnsignedValidator>(source, &dispatch_info, encoded_len)
                }
                _ => Err(InvalidTransaction::Call.into()),
            }
        }
    }
}

pub type CheckedOf<E, C> = <E as Checkable<C>>::Checked;
struct Executive<T>(sp_std::marker::PhantomData<T>);

impl<T: Config> Executive<T>
where
    T::RuntimeCall: Dispatchable<Info = DispatchInfo, PostInfo = PostDispatchInfo>,
{
    fn apply_extrinsic(
        uxt: Box<T::Extrinsic>,
        address_seed: AccountIdLookupOf<T>,
    ) -> DispatchResultWithPostInfo {
        let encoded = uxt.encode();
        let encoded_len = encoded.len();

        // Verify that the signature is good.
        let mut xt = uxt.check(&T::Context::default()).expect("process ?");
        xt.replace_sender(T::Lookup::lookup(address_seed).expect("lookup should succeed"));

        let dispatch_info = xt.get_dispatch_info();
        let r = Applyable::apply::<T::UnsignedValidator>(xt, &dispatch_info, encoded_len)
            .map_err(Error::<T>::from)?;

        // For we has checked the `dispatch_info.class` in `validate_unsigned`, so the check at here is not
        // necessary. We keep this to be same implementation in `Executive`.
        if r.is_err() && dispatch_info.class == DispatchClass::Mandatory {
            return Err(Error::<T>::InvalidTransaction.into())
        }

        r
    }
}

impl<T: Config> From<TransactionValidityError> for Error<T> {
    fn from(value: TransactionValidityError) -> Self {
        match value {
            TransactionValidityError::Invalid(_) => Error::InvalidTransaction,
            TransactionValidityError::Unknown(u) => match u {
                UnknownTransaction::CannotLookup => Error::UnknownTransactionCannotLookup,
                UnknownTransaction::NoUnsignedValidator => {
                    Error::UnknownTransactionNoUnsignedValidator
                }
                UnknownTransaction::Custom(_) => Error::UnknownTransactionCustom,
            },
        }
    }
}