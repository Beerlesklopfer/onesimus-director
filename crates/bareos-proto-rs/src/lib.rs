// Generated protobuf/gRPC types for the Bareos Director service.

pub mod bareos {
    pub mod director {
        tonic::include_proto!("bareos.director");

        /// Command parameter messages (director_commands.proto)
        pub mod commands {
            tonic::include_proto!("bareos.director.commands");
        }

        /// Typed Director API (director_api.proto)
        pub mod api {
            tonic::include_proto!("bareos.director.api");
        }
    }

    pub mod ccc {
        tonic::include_proto!("bareos.ccc");
    }

    // bareos_options.proto extends google.protobuf descriptors;
    // the extensions are available via prost descriptor reflection.
}

pub mod typed;
