use std::marker::PhantomData;

use crate::matmul::components::batch::shared::gmm_execute;
use crate::matmul::components::global::args::{TensorInput, TensorOutput};
use crate::matmul::components::{
    batch, config::MatmulConfig, global, Ident, MatmulKernel, MatmulLaunch, StageDim,
};
use crate::matmul::components::{InputRuntimeArg, MatmulProblem, MatmulSpec, OutputRuntimeArg};
use crate::matmul::kernels::matmul::AdvancedConfig;
use crate::matmul::kernels::MatmulAvailabilityError;
use cubecl_core as cubecl;
use cubecl_core::prelude::*;

use super::{Config as _, CubeDispatch};

/// Executes matrix multiplication at the batch level,
/// assigning each cube to a single global matmul.
///
/// Note: This algorithm requires one cube per global matmul;
/// insufficient cubes will result in incomplete computations.
pub struct Matmul<MS: MatmulSpec, GMM: global::Matmul<MS>, C: CubeDispatch> {
    _ms: PhantomData<MS>,
    _gmm: PhantomData<GMM>,
    _c: PhantomData<C>,
}

#[cube]
impl<MS: MatmulSpec, GMM: global::Matmul<MS>, C: CubeDispatch> batch::Matmul<MS>
    for Matmul<MS, GMM, C>
{
    fn execute(
        lhs: TensorInput<MS::EG, MS::Args>,
        rhs: TensorInput<MS::EG, MS::Args>,
        out: TensorOutput<MS::EG, MS::Args>,
        #[comptime] config: Self::Config,
    ) {
        let (x_index, y_index) = C::x_y_indices();
        let x_offset = x_index * config.stage_dim(Ident::Lhs).num_elements_x_dim();
        let y_offset = y_index * config.stage_dim(Ident::Rhs).num_elements_y_dim();
        let nth_batch = C::batch_index();
        let rank = lhs.rank();
        let k_range = (0, lhs.shape(rank - 1));

        let gmm_config = config.to_gmm_config();
        gmm_execute::<MS, GMM>(
            lhs,
            rhs,
            out,
            x_offset,
            y_offset,
            nth_batch,
            &mut GMM::init_accumulator(gmm_config),
            k_range,
            gmm_config,
        );
    }
}

impl<MS: MatmulSpec, GMM: global::Matmul<MS>, C: CubeDispatch> MatmulKernel for Matmul<MS, GMM, C> {
    type Config = Config<GMM::Config, C>;

    fn check_config(config: Self::Config) {
        GMM::check_config(config.to_gmm_config())
    }

    fn check_availability<R: Runtime>(
        client: &ComputeClient<R::Server, R::Channel>,
    ) -> Result<(), MatmulAvailabilityError> {
        GMM::check_availability::<R>(client)
    }

    fn make_config(
        problem: &MatmulProblem,
        cube_dim: &CubeDim,
        cube_count: &CubeCount,
        advanced_config: &AdvancedConfig,
    ) -> Self::Config {
        let gmm_config = GMM::make_config(problem, cube_dim, cube_count, advanced_config);
        let cube_count = if let CubeCount::Static(x, y, z) = cube_count {
            (*x, *y, *z)
        } else {
            panic!("Dynamic cube count unsupported")
        };

        Config::<GMM::Config, C>::new(gmm_config, cube_count)
    }
}

impl<MS: MatmulSpec, GMM: global::Matmul<MS>, C: CubeDispatch> MatmulLaunch<MS>
    for Matmul<MS, GMM, C>
{
    unsafe fn launch_unchecked<'a, R: Runtime>(
        client: &ComputeClient<<R as Runtime>::Server, <R as Runtime>::Channel>,
        cube_dim: CubeDim,
        cube_count: CubeCount,
        input: InputRuntimeArg<'a, MS, R>,
        output: OutputRuntimeArg<'a, MS, R>,
        config: Self::Config,
    ) {
        Self::check_config(config);
        super::batch_matmul::launch_unchecked::<MS, Self, R>(
            client, cube_count, cube_dim, input, output, config,
        );
    }
}

#[derive(CubeType, Copy, Clone, Debug, Hash, PartialEq, Eq)]
/// Configuration for the OneToOneBatchMatmul
pub struct Config<G: global::Config, C: CubeDispatch> {
    gmm_config: G,
    cube_count: (u32, u32, u32),
    _c: PhantomData<C>,
}

impl<G: global::Config, C: CubeDispatch> batch::Config for Config<G, C> {
    type GmmConfig = G;

    fn to_gmm_config(&self) -> Self::GmmConfig {
        self.gmm_config
    }

    fn stage_dim(&self, ident: Ident) -> Box<dyn StageDim> {
        self.gmm_config.stage_dim(ident)
    }

    fn max_m(&self) -> u32 {
        C::max_x(self.cube_count) * self.stage_dim(Ident::Out).num_elements_x_dim()
    }

    fn max_n(&self) -> u32 {
        C::max_y(self.cube_count) * self.stage_dim(Ident::Out).num_elements_y_dim()
    }

    fn max_batches(&self) -> u32 {
        C::max_batches(self.cube_count)
    }
}

impl<G: global::Config, C: CubeDispatch> MatmulConfig for Config<G, C> {}

impl<G: global::Config, C: CubeDispatch> Config<G, C> {
    pub fn new(gmm_config: G, cube_count: (u32, u32, u32)) -> Self {
        Self {
            gmm_config,
            cube_count,
            _c: PhantomData,
        }
    }
}
