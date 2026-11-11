import { ObjectList } from "./ObjectList";

export function Workspace() {
  return (
    <>
      <div className="tabs">
        <div className="tab active"><span className="title">Objects</span></div>
      </div>
      <div className="tab-body">
        <ObjectList />
      </div>
    </>
  );
}
