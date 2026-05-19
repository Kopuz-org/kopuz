{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

{
  cachix.pull = [ "kopuz" ];
  cachix.push = "kopuz";
}
